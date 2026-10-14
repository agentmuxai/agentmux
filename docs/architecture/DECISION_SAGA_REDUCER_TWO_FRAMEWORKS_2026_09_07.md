# Decision: the launcher and srv saga/reducer frameworks stay separate, and say so

**Status:** active — accepted by the repo owner 2026-09-07. Option 3: the two frameworks stay separate and each module header says so. The cross-references shipped in the PR that flipped this line; no code moved. Verified 2026-09-07.

## Context

Both `agentmux-launcher` and `agentmux-srv` have a module called `saga` (`sagas` in srv) and a module called `reducer`. The audit measured the pair at about 11K lines sharing exactly one method name (`new`) between the two saga `mod.rs` files, and called the state — same names, undocumented divergence — the only bad option: either there is one abstraction that belongs in a crate both depend on, or there are two and the shared vocabulary should stop implying otherwise.

What is actually there (read 2026-09-07):

| | launcher | srv |
|---|---|---|
| saga module | `src/saga/mod.rs`, 1,858 lines (Phase E.1a / F.5) | `src/sagas/mod.rs`, 539 lines, plus one file per saga (Phase E.5.5) |
| saga shape | a `Saga` **trait** — a state machine the `SagaCoordinator` drives from a **bus subscription**: trigger events start sagas, later bus events route in via `on_event`, results come back as `SagaAction`s, `SagaStarted` / `SagaCompleted` / `SagaFailed` bracket the run | **async functions**: `alloc_saga_id` → `emit_saga_started` → drive the steps with `SagaCtx::dispatch` / `compensate` (in-process oneshot dispatch into the reducer) → `emit_terminal`; `run_saga` wraps the future |
| consumers | one (`pool_respawn`) | every workspace/tab/block multi-step command (tear-off, restore, move, delete) |
| reducer | `src/reducer/mod.rs`, 631 lines; `update(&mut State, Command, &Ctx) -> Vec<Event>` over the window/process state machine ([`docs/specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md`](../specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md)) | `src/reducer.rs`, 314 lines; the same signature over workspace/tab/block state (Phase E) |

The srv module's own header already explains why it is not the launcher framework: the Phase E plan assumed sagas would fan out across host, launcher and srv over IPC; the implementation kept that fan-out in the frontend, so every srv saga mutates only srv state, and an in-process oneshot beats an IPC round-trip on every step ([`docs/retro/saga-coordinator-location-analysis-2026-04-30.md`](../retro/saga-coordinator-location-analysis-2026-04-30.md)). The launcher framework exists for the cross-process case, which never grew past `pool_respawn`.

The two reducers share a *discipline* — pure, total, deterministic, no I/O, mutex held only during dispatch — stated in both headers in nearly the same words, AND the wire types: both import `agentmux_common::ipc::{Command, Event}`. What they do not share is `State` (each crate's own) or which command/event variants each `update` actually handles.

## Options

1. **One shared crate.** Lift a `Saga` trait plus coordinator and a `Reducer` trait into `agentmux-common` (or a new crate), and port srv's async-function sagas onto the trait. Cost: srv's sagas rewritten as state machines to fit a bus-driven coordinator they do not need, and the in-process oneshot dispatch that was chosen on purpose becomes an event round-trip again. Benefit: one vocabulary means one implementation. Nothing today needs a saga that spans both processes.
2. **Keep both, rename one.** Rename srv's `sagas` to, say, `flows` and `SagaCtx` to `FlowCtx`, so a reader never assumes the launcher framework. Cost: churn across srv, its Phase E specs and retros, and the `SagaStarted` / `SagaFailed` event names, which are on the wire to the frontend. Benefit: the name stops lying.
3. **Keep both, document the split where a reader will hit it.** This document, plus a two-line cross-reference at the top of each of the four modules ("not the same framework as `<other crate>::<module>` — see this decision"). Optionally, a `Reducer` trait in `agentmux-common` that both `update` functions implement, so the shared discipline becomes a compiler-checked contract instead of two prose paragraphs. No code moves, no behaviour changes.

## Recommendation

Option 3, with the optional `Reducer` trait deferred until something would call it generically — a proptest harness that runs both reducers through the same invariant checks would be the first honest consumer.

Reasons: the divergence is designed, not accidental. The srv header and the 04-30 retro already record the trade-off, and the audit's own §2.5 concluded "either outcome is fine; the current state is the only bad one", where the bad part is that the choice was never written down as a choice. Option 1 would undo a deliberate decision in order to remove a duplication that does not exist at the code level. Option 2 pays wire-format and spec churn for a clarity this page provides for free. If a genuinely cross-process saga is ever built — the original Phase E fan-out — that is the moment to revisit option 1, because then the launcher coordinator would have a second consumer and srv a reason to speak its protocol.

## Consequences

- No code changes in this decision. **Done on acceptance:** the four header cross-references landed as a comments-only PR (`agentmux-launcher/src/saga/mod.rs`, `agentmux-srv/src/sagas/mod.rs`, `agentmux-launcher/src/reducer/mod.rs`, `agentmux-srv/src/reducer.rs`), and this page moved to `active`.
- The audit's Phase 5 line item closes as "decided: separate, documented".
- Re-open if a saga needs to coordinate launcher and srv state in one run, or if a third saga or reducer framework appears anywhere in the workspace.

## Addendum 2026-10-14: what the 09-07 reading missed

Found by the [reducer stack triage](../analysis/REPORT_REDUCER_STACK_TRIAGE_2026_10_13.md). The decision itself is unchanged; these are facts for whoever weighs the re-open trigger above.

- **There is a third reducer, and it predates this decision.** The host's `crates/cef/src/reducer/` (Phase H, May 2026) has its own `HostCommand` enum (30 variants) and returns a `DispatchOutput` the caller applies as Win32/CEF effects, instead of `Vec<Event>` over the shared `ipc::Command`. It isn't a saga framework, and it doesn't share the launcher/srv signature. Read literally, the trigger "a third saga or reducer framework appears" has already fired. Whether it should re-open option 1 or 3's optional `Reducer` trait is the owner's call.
- **"Pure, total, deterministic, no I/O" was not true of all three.** The launcher logged to a file from inside `apply_hwnd_position_changed`, srv generates UUIDs inside reduce, and the host reducer reads the clock, a global counter and an env var. The triage's §2 lists each one.
- **The sizes have moved on.** srv's reducer is now `crates/srv/src/reducer.rs` (349 lines) plus per-domain submodules (~6.3K lines with tests). The paths above predate the 2026-10 move of the crates to `crates/<name>/`.
