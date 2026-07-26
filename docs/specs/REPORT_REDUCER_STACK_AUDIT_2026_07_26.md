# Reducer Stack Audit — Post-Mortem on Duplication, Coupling, and Modularization

**Date:** 2026-07-26
**Author:** AgentA
**Status:** Report — analysis only, no code changes. Synthesizes 6 parallel research passes.
**Ground truth basis:** `agentmuxai/agentmux` `main` at commit `35af4958`, pulled fresh for this report.
**Scope note (read this first):** [`docs/reports/REPORT_LARGE_FILE_MODULARIZATION_SCAN_2026_07_22.md`](../reports/REPORT_LARGE_FILE_MODULARIZATION_SCAN_2026_07_22.md)
already did a systematic 84-file scan answering *"should this file be split?"* — it explicitly scopes itself as
"structural... proposed splits, not a redesign... nothing here changes behavior." That report already covers
`agentmux-srv/src/reducer/{layout,tab}.rs`'s file-size question (both already got their prescribed
test-colocation split). **This report does not re-answer "should file X be split."** It answers a different
question that report didn't ask: *is the same logic reimplemented more than once across the reducer stack,
and where does state get tracked by more than one mechanism that should be one?* The two reports are
complementary, not competing — cite the large-file scan for "where should this code live," cite this one for
"is this code duplicated, and should these two state machines be one."
**Related:**
- [`REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md`](REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md)
  — six process-liveness mechanisms + a proposed Process Broker with a reducer-governed core. This report's
  findings directly bear on that proposal's design (§6 below).
- [`REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md`](REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md) and
  the Credential Broker it produced (`agentmux-srv/src/broker/`) — this report extends that domain's audit
  down to two backend session managers that report didn't examine (§4).
- [`docs/specs/PLAN_SRV_REDUCER_MODULARIZATION_2026-05-07.md`](PLAN_SRV_REDUCER_MODULARIZATION_2026-05-07.md)
  — the original plan that gave `agentmux-srv`'s reducer its current domain-submodule shape. Confirms the
  three-reducer architecture (srv/launcher/cef) was already recognized as a deliberate, repeated pattern as
  of 2026-05-07 — this report is the first pass checking whether that repetition shares code or just shape.

---

## 0. Executive summary

AgentMux has, depending on how you count, somewhere between **7 and 10 independent state-machine/reducer
implementations**, split across three Rust processes and the frontend, that were each built correctly for
their own moment but never cross-checked against each other for duplication. The pattern that recurs across
every one of the six parallel research passes behind this report is the same shape, found independently each
time: **a convention gets repeated by hand instead of by code-sharing.** Three Rust reducers (`agentmux-srv`,
`agentmux-launcher`, `agentmux-cef`) all implement the identical `update(&mut State, Command) -> Vec<Event>`
discipline — correctly, by design, per `PLAN_SRV_REDUCER_MODULARIZATION_2026-05-07.md` — but with zero shared
crate or trait behind it: the same `Ctx` struct, the same error-arm shape, the same version-bump call, and the
same domain-submodule extraction pattern were each typed out independently, three times, sometimes with the
*same 5-line boilerplate block copy-pasted 50+ times within a single one of those three*. The frontend's auth
domain shows the identical failure mode one layer up: two backend session managers
(`AuthSessionManager`/`OAuthSessionManager`) and two frontend login-flow implementations
(`AuthState`/`LaunchPhase`) that are ~90-100% conceptually identical and ~0% code-shared, built months apart
by different authors solving the same problem from scratch each time.

None of this is "everything is spaghetti." Three of the six passes independently confirmed specific reducers
are **well-factored and worth copying as the house pattern** — `agentmux-srv`'s Phase E reducer (self-contained,
zero TODO markers, a genuinely sophisticated `apply_atomic` clone-mutate-commit helper) and the frontend's
`agent-pane-state/reducer.ts` (145 tests, injectable time, exhaustive unions, PR-provenance comments on every
non-obvious branch) are both real engineering, not accidents. The finding isn't that the codebase can't write
a good reducer — it's that every time it writes a *second* one for an adjacent problem, it doesn't reuse the
first one's infrastructure, only its shape.

**The standing question this report was explicitly asked to resolve** — "why isn't `AuthState` and
`CredentialState` one reducer" — has a decisive answer in §4.4: **no, don't merge them** (they govern
genuinely different lifecycles across a process boundary), **but two specific pairs inside that same cluster
should merge**, and the amount of hand-duplicated boilerplate *inside* the correctly-separate pieces is the
real problem, not the separation itself.

---

## 1. Full inventory — every reducer/state-machine found

| # | Mechanism | Process/Layer | Governs | Pattern |
|---|---|---|---|---|
| 1 | `agentmux-srv` Phase E reducer | Backend (srv) | Window/tab/workspace/block lifecycle, IPC-client `ProcessRecord` | Formal `update(&mut State, Command, &Ctx) -> Vec<Event>` |
| 2 | `agentmux-launcher` reducer | Launcher | Connection registry, window mirror, warm-pool mirror, WRR telemetry | Same formal shape, mutates `&mut State` in place (diverges from its own spec doc) |
| 3 | `agentmux-launcher` saga coordinator | Launcher | Multi-step post-launch flows (pool respawn, window-cleanup cascade) | A **second**, independently-invented `Saga` trait (`Phase` enum + `on_event -> SagaAction`) — same job as #2, zero shared code |
| 4 | `agentmux-cef` host reducer | CEF host | OS/CEF-process-local window-manager state (panes, pools, drag, opacity, quit) | Same formal shape, bespoke `DispatchOutput` (not a bare event vec) — justified adaptation |
| 5 | `agentmux-cef`'s `UiThreadGate` | CEF host | Main-thread-ready / snapshot-received gating | A **fourth**, separate small pure-state-machine outside the main `HostState`/`host_dispatch` path entirely |
| 6 | Frontend `agent-pane-state` reducer | Frontend | Per-turn `TurnPhase` (Idle/Submitting/Streaming/Interrupting/Done/Disconnected) | Formal `update(state, command, nowMs)`, the most mature reducer in the whole audit |
| 7 | Frontend `agent-document`/`browser-pane-state`/`agent-pane-layout`/`launcher-event` reducers | Frontend | Document/session lifecycle, browser tabs, pane layout, launcher-event projection | Same house convention, cleanly non-overlapping domains |
| 8 | Frontend `AuthState` (pre-launch modal) | Frontend | Bundle/identity selection + OAuth *before* a pane/instance exists | Formal reducer, idempotency + stale-dispatch guards baked in |
| 9 | Frontend `LaunchPhase`/`useAgentControllerStatus` cluster | Frontend | Post-mount pane's login/launch sub-phases | **Not** a reducer — a discriminated union + ad hoc local flags scattered across 2 files |
| 10 | Backend `CredentialState`/`RefreshScheduler` (Credential Broker) | Backend (srv) | Per-credential proactive refresh coordination (MuxBus only today) | Formal reducer-governed broker, the newest and most disciplined of the backend mechanisms |
| 11 | Backend `AuthSessionManager` | Backend (srv) | Pre-launch-modal CLI-provider OAuth sessions (600s timeout) | `Arc<Mutex<HashMap>>` on `AppState`, DI'd |
| 12 | Backend `OAuthSessionManager` | Backend (srv) | Armory service-account OAuth (unshipped scaffold, 300s timeout) | Process-global `OnceLock`, **structurally near-identical to #11** |
| 13 | Backend spawn-gate (`inject_identity_env_with_broker`) | Backend (srv) | Synchronous, fail-closed admission check at CLI spawn time | Not stateful — a decision function, correctly distinct from #10-12 |
| 14 | Backend `subagent_watcher` | Backend (srv) | Subagent/dispatch status (Active/Completed/Abandoned) | Small, lock-protected, reducer-*like* but not formalized — confirmed low-risk as-is (§5) |

Fourteen numbered mechanisms is the honest count once the saga coordinator and `UiThreadGate` are counted as
the separate state machines they actually are — not the "3 backend + a handful of frontend" mental model the
codebase's own doc comments imply.

---

## 2. The Rust reducer trio: same shape, zero shared code

All three backend reducers (`agentmux-srv`, `agentmux-launcher`, `agentmux-cef`) were verified, independently,
to share the exact function signature family:

```rust
// agentmux-srv/src/reducer.rs:49
pub fn update(state: &mut State, cmd: Command, ctx: &Ctx) -> Vec<Event>
// agentmux-launcher/src/reducer/mod.rs:84
pub fn update(state: &mut State, cmd: Command, ctx: &Ctx) -> Vec<Event>
// agentmux-cef/src/reducer/mod.rs:1024
pub fn update(state: &mut HostState, cmd: HostCommand) -> DispatchOutput
```

`agentmux-srv/src/reducer.rs`'s own module doc says this is deliberate: *"Same discipline as
`agentmux-launcher::reducer`."* That's true as a **convention** — verified: there is no shared trait or crate
behind it. `agentmux-common` defines the wire schema (`Command`/`Event`/`ErrorCode`) but nothing named
`Reducer`/`ReducerCore` exists anywhere in the workspace (zero grep hits). Each of the three crates
independently hand-writes:

- The `Ctx` struct (`now_rfc3339`/`conn_id`/`registered_pid`, near-identical in srv and launcher).
- The version-bump call (`state.bump_version()`, ~80 call sites in srv alone; `bump_version()` in launcher too).
- The `Event::Error{ code: InvalidCommand, ... }` rejection-arm shape for out-of-domain commands.

That third one is where the duplication stops being merely "the same idea, typed twice" and becomes literal
copy-paste at scale:

- **`agentmux-srv`**: the 5-line `Event::Error{...}` block appears **~50 times** across `layout.rs` (20),
  `tab.rs` (20), `block.rs` (3), `window.rs` (3), `workspace.rs` (2), `reducer.rs` (2). Only `layout.rs`
  factored part of it out — `unknown_tab()`/`op_error()` (`reducer/layout.rs:482-501`) — but that extraction
  landed partway through the file's own history and was **never applied retroactively** to the ~10 handlers
  written before it (`handle_set_focused_node` at `layout.rs:14-23`, `handle_set_magnified_node` at `:39-48`,
  `handle_layout_clear` at `:66-73`, `handle_layout_set_tree` at `:96-103`, plus 5 more inline sites in
  `handle_layout_insert_node`). `tab.rs`/`block.rs`/`workspace.rs`/`window.rs` have **zero** such helpers at
  all — every handler hand-rolls the identical validation block from scratch.
- **`agentmux-launcher`**: the *same* rejection shape is repeated **~20 times** in `mod.rs:325-560` as
  individual match arms, where `agentmux-srv/src/reducer.rs:274-282` already demonstrates the fix — a single
  catch-all `other => {...}` arm. Launcher's version is strictly worse than srv's own pattern, in the same
  codebase, unfixed.

**Recommendation:** extract a tiny shared helper — even just a free function
`fn invalid_command_error(cmd_name: &str, version: u64) -> Event` in `agentmux-common` — that all three
reducers import. This is the single cheapest, lowest-risk win in the entire audit: no behavior change, ~70
duplicated call sites collapse to imports of one function, and it directly fixes launcher's worse-than-srv's-
own-precedent regression without anyone having had to notice it by inspection.

### 2.1 Two more reducer-shaped things hiding next to the "real" ones

- **`agentmux-launcher/src/saga/mod.rs`** implements a `trait Saga` (`Phase` enum + `on_event(&Event, &SagaCtx)
  -> SagaAction`, `mod.rs:178-210`) to drive `pool_respawn`/`window_cleanup_cascade`. This is functionally
  `(State, Event) -> Action` — the mirror image of the reducer's `(State, Command) -> Event` — invented
  independently, with its own dispatch loop (`run_coordinator`, `mod.rs:904-1088`), own state
  (`in_flight: HashMap<u64, InFlightSaga>`), and zero shared code with `reducer::update`. It composes cleanly
  at the boundary (sagas only see bus events, emit commands back through the pipe) so this isn't tangled —
  but it is a second state-machine discipline solving the same conceptual problem the "reducer" convention
  already names, built from scratch instead of extending it.
- **`agentmux-cef/src/state/ui_thread_gate.rs`**'s `UiThreadGate` (`on_main_ready`/`on_snapshot`,
  lines 85-125) is a small, well-isolated, unit-tested pure state machine that lives **entirely outside**
  `HostState`/`host_dispatch` — driven directly by `client/lifecycle.rs`/`launcher_ipc.rs`/
  `commands/window/meta.rs` instead of going through the CEF reducer that already exists in the same process.
  Low-risk on its own, but it means one process now has *two* different reducer-pattern constructs with two
  different dispatch mechanisms, discoverable only by reading both files — worth folding into the main
  reducer's `Command`/`Event` vocabulary the next time either is touched, not urgent on its own.

### 2.2 One genuine purity violation, one dormant subsystem

`agentmux-cef/src/reducer/browsers.rs:99-104` calls `tracing::info!(...)` directly inside
`handle_relabel_browser` — the module's own doc comment at `mod.rs:1190-1194` states the rule explicitly ("no
logging — logging happens in `state::log_host_event` after dispatch returns"). One isolated instance (grep
confirms no other `tracing::` calls anywhere in `reducer/`), not a pattern, but a real, citable discipline
breach — a one-line fix (move the log call to the post-dispatch site).

`agentmux-cef/src/reducer/top_level.rs` (249 lines) implements the full H.6 top-level-window-creation runner
(4 commands, 5 events) but is explicitly marked dormant in the module's own doc comment
(`mod.rs:127-133`): *"no production code dispatches to them. The `ui_tasks::post_create_window` direct-call
path is still authoritative."* ~250 lines of `#[allow(dead_code)]`-flagged, fully-built-but-unwired subsystem.
Either finish wiring it up or delete it — leaving it half-migrated indefinitely is the worst of both options
(it bit-rots silently, and a future reader has to determine it's inert before trusting it).

### 2.3 The shared `Command`/`Event` enum is a vocabulary-coupling smell

`Command`/`Event` are single flat enums in `agentmux-common/src/ipc.rs` (127 variants total), shared across
all three Rust reducers. Each reducer only meaningfully handles a subset (srv: ~40 variants) and routes
everything else through its own catch-all rejection. This means **the vocabulary isn't reducer-scoped**: a
new host-only `Command` variant forces every non-host reducer to either add a no-op arm or silently fall
through a catch-all it didn't ask to be aware of. This is the structural root cause behind both the ~50 and
~20 duplicated rejection-arm counts in §2 — if each reducer only compiled against its own scoped subset of
commands, there would be nothing to reject in the first place. Worth flagging explicitly for whoever designs
the Process Broker (`REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK`'s proposed fourth reducer, §6 below): a
fourth reducer onto the same flat enum repeats this exact problem a fourth time.

---

## 3. Frontend pane-state reducer family — mostly clean, two concrete fixes

The frontend reducer family (`agent-pane-state`, `agent-document`, `browser-pane-state`, `agent-pane-layout`,
`launcher-event`) is **structurally sound** — verified zero cross-imports between sibling reducers'
`reducer.ts`/`types.ts` files, coordination happens one layer up in `agent-pane-model.ts` via explicit
per-slice dispatch methods (`dispatchPane()`/`dispatchDoc()`), never auto-forwarding between reducers. This is
the one cluster in the audit that already looks the way the Rust trio should.

Two concrete, low-risk items:

1. **The `launcher-event` naming collision is not a duplicate** (initial hypothesis, disproven) —
   `launcher-event/reducer.ts` (365 lines) is the pure core (`update(state, command)`, zero I/O, zero SolidJS);
   `launcher-event-reducer.ts` (340 lines) imports `update` from it and wraps the impure shell (module-level
   state cell, `createEffect` subscription, RPC resync, public API). This is the same pure-core/impure-shell
   split every sibling slice uses (a `<slice>/reducer.ts` + a `<slice>-store.ts` wrapper) — just with
   confusing file naming (a directory `launcher-event/` sitting next to a same-stem flat file). **Fix: rename**
   `launcher-event-reducer.ts` → `launcher-event-store.ts` (matching every sibling's `*-store.ts` convention)
   to stop it reading as dead code to future auditors. Do not delete or merge anything — the architecture
   here is correct.
2. **`browser-pane-state/reducer.ts`'s tab-splice pattern is repeated ~9 times** (`TabUrlChanged`,
   `TabTitleChanged`, `TabFaviconChanged`, `TabLoadingChanged`, `Navigate`, `LoadStarted`, `LoadFinished`,
   `HistoryUpdated`, `UrlConfirmed`, `FaviconUrlsReceived` — line ranges in the sub-report) — find tab index,
   clone tab, splice a rebuilt array back in — despite the file already defining a `replaceTab()` helper
   (line 61) that only 2 of ~13 tab-mutating arms actually use. **Fix: route the other ~9 arms through
   `replaceTab()`** — an estimated 80-100 line reduction, purely mechanical, zero behavior change.

`agent-pane-state/reducer.ts`'s reputation as the house exemplar is earned, not assumed: 145 tests (the
largest suite in the family by a wide margin), a pure function that takes `nowMs` as an injectable parameter
(rather than reading `Date.now()` internally, which is what makes its 122+ timing-sensitive tests
deterministic), an exhaustive discriminated-union switch with 7 numbered invariants stated in the file header,
and every non-obvious branch citing the specific PR/issue that motivated it. This is the pattern worth holding
up codebase-wide — not just "write a reducer," but "write a reducer with injectable time, an audit event for
every suppressed no-op, and inline provenance for every non-obvious guard."

---

## 4. The auth/credential cluster — the decisive answer

This is the densest part of the audit and the part the user asked about directly. Ten of the fourteen
mechanisms in §1's inventory live here. The full per-mechanism inventory is in the appendix; this section is
the verdict.

### 4.1 `AuthSessionManager` vs `OAuthSessionManager` — confirmed duplicate, should merge

```rust
// agentmux-srv/src/identity/auth_session.rs
pub struct AuthSessionManager { sessions: Arc<Mutex<HashMap<String, Session>>>, process_refs: Arc<Mutex<ProcessRefs>> }
// agentmux-srv/src/identity/oauth_client.rs
pub struct OAuthSessionManager { sessions: Mutex<HashMap<String, OAuthSession>> }
```

Both: `new_session`/`start_session` minting a `{prefix}-{uuid}` id, a `status` enum with the identical
`Pending → UrlAvailable/CodeEmitted → Success/Failed` shape, an `is_terminal()` helper, a hand-timed lazy
sweep on poll, a `cancel` that force-transitions to `Failed`. `oauth_client.rs`'s own doc comment even names
the sibling explicitly ("Distinct from the CLI-provider OAuth in `auth_session.rs`"). The one real structural
difference — `AuthSessionManager` is DI'd via `AppState` (testable), `OAuthSessionManager` is a raw
`OnceLock` global (not) — is itself worth fixing regardless of merging.

**This is accidental duplication, not architectural**, confirmed by timeline: `auth_session.rs` shipped for
`SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14`; `oauth_client.rs` is a later, still-unshipped Armory scaffold
(`client_id: None` everywhere, 4 `TODO: provision public client id` markers). The one real domain asymmetry
— `AuthSessionManager` also tracks OS process handles (`drain_tasks`/`stdin_senders`/`pty_pids`) because it
manages a spawned CLI subprocess, `OAuthSessionManager` doesn't need that because it drives the HTTP/browser
flow directly — is naturally modeled as an *optional* extension on a shared generic type, not a reason for
two independent implementations.

**Recommendation:** extract one generic `TimedSessionMap<S: SessionStatus>` (start/poll/cancel/timeout/prune)
that both call sites use; the CLI-provider caller attaches the process-handle bookkeeping as an extra field.
Low risk — `OAuthSessionManager`'s only caller (the Armory OAuth scaffold) is unshipped and inert.

### 4.2 `AuthState` vs `LaunchPhase`/`launch-flow.ts` — same concept, ~0% shared code

These are the two frontend mechanisms the user asked about by name. Verified zero import relationship between
them — the only linkage is a single doc-comment cross-reference in `launch-phase.ts:9`. Vocabulary overlap is
close to total:

| Concept | `AuthState` | `LaunchPhase` |
|---|---|---|
| Not yet authenticated | `kind: "unauthenticated"` | `checking-auth` → `first-login`/`auth-expired` |
| Waiting on OAuth/CLI round trip | `kind: "waiting"` + `sessionId`/`authUrl` | `waiting-for-login-link`/`opening-login-terminal`/`waiting-for-login-completion` (each with `deadlineMs`) |
| One more step after auth | `kind: "authenticated"` → `"saving"` | `verifying` |
| Done | `kind: "ready"` | `fresh-ready`/`resumed-ready` |
| Failed | `kind: "failed"` + `error` | `failed` + `reason` |

Same idea (idle → checking → waiting-with-a-timer → done-or-failed), independently typed twice, with
different field names for the same concepts (`error` vs `reason`) and no shared timeout constant. `AuthState`
is a real reducer with idempotency/stale-dispatch guards built into every arm; `LaunchPhase` is a plain
discriminated union whose equivalent guarding logic (stale-poll rejection, cancel races, double-fire
prevention) instead lives as scattered ad hoc booleans (`reloginInFlight`, `seedInFlight`, `loginCancelled`)
across two different files, with **no action-token-style guard at all** — a real, if lower-severity, gap
relative to `AuthState`'s own discipline.

### 4.3 The triplicated poll loop

Three independent hand-rolled loops answer the identical question ("has `CheckCliAuth` flipped to
authenticated"): `auth-flow-controller.ts:555-599` (1000ms, has real staleness/cancellation handling),
`launch-flow.ts:346-362` (2000ms, raw `while` loop), `useAgentControllerStatus.ts:403-419` (2000ms, **byte-
identical** raw `while` loop to the previous one). None share a helper. This is unextracted duplication, not
a design question — one `pollUntilAuthenticated()` helper replaces two of the three call sites outright.

Timeout constants tell the same story: `LOGIN_LINK_CAPTURE_LABEL_MS = 15_000` (`launch-phase.ts:24`) is
*already flagged in its own doc comment* as manually kept in sync with Rust's `URL_CAPTURE_TIMEOUT_SECS` in
`cli_login.rs` — an acknowledged, unfixed duplication. The 5-minute auth deadline is hardcoded identically
in two files. Backend has three unrelated timeout constants for the same underlying question ("how long can
an auth attempt hang before giving up"): `SESSION_TIMEOUT_SECS = 600` (`auth_session.rs`), `SESSION_TIMEOUT =
300s` (`oauth_client.rs`), `NEEDS_REAUTH_THRESHOLD = 5` (`broker/state.rs`) — three numbers, three files, no
shared constant module.

### 4.4 The decisive answer

**No, do not merge `AuthState`, `LaunchPhase`, `CredentialState`, `AuthSessionManager`, and the spawn gate
into one reducer.** Three separations are real and should stay:

- **`AuthState` vs `LaunchPhase`**: different *objects*, different *points in time*. `AuthState` runs
  **before an agent pane/instance exists** (no block_id, no controller) — it's selecting/creating an identity
  bundle. `LaunchPhase` runs **after the pane is mounted**, gating an already-selected identity's CLI spawn.
  Merging would force the pre-launch modal to fabricate a fake pane context, or the post-mount flow to
  re-implement bundle selection. Give them a **shared library** for the genuinely identical parts (see below),
  not a shared reducer for the parts that structurally differ.
- **Frontend vs backend** (`AuthState` vs `AuthSessionManager`/`CredentialState`): different processes across
  an RPC boundary. A reducer cannot span that boundary without becoming the serialization protocol the RPC
  layer already is. Correct and unavoidable.
- **`RefreshScheduler`/`CredentialState` vs the spawn gate**: already correctly separated per
  `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK`'s own §3.0.3 finding — proactive, schedule-driven refresh
  vs. synchronous, fail-closed, one-shot admission check. Different risk profiles, different triggers.
  Confirmed still correct.

**But merge the accidental duplicates**: `AuthSessionManager` + `OAuthSessionManager` (§4.1, near-zero risk),
and the triplicated poll loop (§4.3, pure extraction). And recognize the real shape of the problem: **it's
effectively 5 frontend mechanisms doing the work 2 well-factored ones should** — a pre-launch reducer and a
post-mount reducer, sharing one poll/timeout/cancellation library between them — not 5 mechanisms because the
domain genuinely needs 5. The fragmentation is fine; the unshared boilerplate living *inside* the
fragmentation is the actual bug.

`runLaunchFlow` (422 lines, 8 distinct concerns: container detection, CLI resolve, auth-check,
existing-account lookup, tier-1/2/3 dispatch, recheck-poll, persistence, controller resync) and
`useAgentControllerStatus.relogin` (184 lines) are a near-full duplicate of the same tier-1/2/3-dispatch +
recheck-poll + persist logic, differentiated only by which UI affordance triggered it — a third place this
same login-attempt logic is independently implemented (alongside `runProviderLogin`'s own tier machinery).
Worth a follow-up: extract the shared "attempt a login and confirm it landed" core once, called from both the
mount-time flow and the recovery flows.

---

## 5. Subagent lifecycle — the "no reducer" claim is stale, resolved

A 2026-07-16 report claimed subagent state had "no reducer, no liveness." Verified: **no longer true.**
`subagent_watcher.rs` was split into a module (`subagent_watcher/{mod,types,scan,jsonl,query,parse}.rs`) by a
later modularization PR (#2283), then hardened further (#2286, ~2026-07-26). State mutation is now a small,
disciplined set of lock-protected direct writes, each with an inline invariant comment — not a formal reducer,
but not the chaos the original claim implied either: 3 call sites for `SubAgentStatus` (init/Complete/Abandon),
1 recompute function for `DispatchStatus` called from 3 places (which is itself a legitimately reducer-like
pattern — full recomputation from source fields rather than incremental mutation).

One real gap: `jsonl.rs:180`'s `Completed` write is unconditional and would silently overwrite an `Abandoned`
status if a late JSONL write arrived after reconciliation — `scan.rs:141`'s own guard only protects the other
direction (`Active → Abandoned` refuses to touch an already-`Completed` subagent). **Recommendation: a
one-line guard** (`if status != Abandoned` before the Completed write), not a reducer proposal — state here is
genuinely small enough (2 enums, 3-4 mutation sites, already self-correcting toward recomputation for the one
field that needed it) that building a formal reducer would be over-engineering relative to the actual risk.

---

## 6. Bearing on the proposed Process Broker

`REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md` §3.1 recommends the future Process Broker be
built as a fourth reducer, explicitly modeled on `agentmux-srv/src/reducer.rs`'s "pure `update`, single
Mutex-guarded dispatch" shape. This audit's findings sharpen that recommendation with two concrete warnings
that report couldn't have had, since it predates this one:

1. **Don't let it become a fourth hand-reimplementation of the same boilerplate** (§2) — if a shared
   `invalid_command_error`-style helper or a genuine `ReducerCore` abstraction gets extracted per this
   report's top recommendation, the Process Broker should be the first *consumer* of it, not the fourth crate
   to reinvent the `Ctx`/version-bump/error-arm shape from scratch.
2. **Don't put it on the same flat `Command`/`Event` enum** (§2.3) — a fourth reducer sharing that enum
   repeats the vocabulary-coupling problem a fourth time. If the Process Broker needs its own Command/Event
   vocabulary, scope it to its own enum from day one rather than adding N more variants to the existing
   127-variant one and N more catch-all rejection arms to the other three reducers.

---

## 7. Prioritized recommendations

**P0 — near-zero risk, do first:**
1. Extract one shared `invalid_command_error()` helper for the ~70 duplicated rejection-arm sites across
   `agentmux-srv`/`agentmux-launcher` (§2).
2. Merge `AuthSessionManager` + `OAuthSessionManager` into one `TimedSessionMap<S>` (§4.1) — the only caller
   of one side is unshipped scaffold code.
3. Extract the triplicated `pollUntilAuthenticated()` loop (§4.3).
4. Fix the one stray `tracing::info!` inside `agentmux-cef`'s pure reducer arm (`browsers.rs:99-104`, §2.2).
5. Add the missing `Completed`-overwrite guard in `subagent_watcher/jsonl.rs:180` (§5).
6. Rename `launcher-event-reducer.ts` → `launcher-event-store.ts` (§3, item 1) — cosmetic, removes a
   recurring "is this dead code?" question for future readers.

**P1 — moderate effort, real value:**
7. Retroactively apply `layout.rs`'s own `unknown_tab()`/`op_error()` helpers to the ~10 handlers predating
   them in the same file (§2).
8. Route `browser-pane-state/reducer.ts`'s remaining ~9 tab-mutating arms through its existing `replaceTab()`
   helper (§3, item 2) — an estimated 80-100 line cut.
9. Decide the fate of `agentmux-cef`'s dormant H.6 top-level-window subsystem (§2.2) — finish wiring it or
   delete it; don't leave ~250 lines half-migrated.
10. Fold `UiThreadGate` into the CEF reducer's main `Command`/`Event` vocabulary the next time either file is
    touched (§2.1) — not urgent standalone, but stop the "two dispatch mechanisms in one process" pattern
    from becoming three.

**P2 — bigger, needs a design decision:**
11. Design a genuinely shared `ReducerCore` abstraction (crate or trait) that `agentmux-srv`/`launcher`/`cef`
    build on instead of hand-reimplementing the same `Ctx`/dispatch/version-bump shape — do this *before* the
    Process Broker becomes a fourth independent implementation (§6).
12. Extract the shared "attempt a login and confirm it landed" core out of `runLaunchFlow` and
    `useAgentControllerStatus.relogin`'s near-duplicate tier-1/2/3 dispatch logic (§4.4).
13. Split the flat `Command`/`Event` enum into per-reducer-scoped vocabularies, or move to a
    trait-based registration model, so adding a host-only command stops forcing every other reducer to
    add a rejection arm for it (§2.3).

---

## 8. What's already right — don't touch these

- `agentmux-srv`'s Phase E reducer's core design: uniform validate→mutate→bump_version→emit skeleton, a
  single choke point for the version counter, cascade-and-carry-ids events, and `apply_atomic`'s
  clone-mutate-commit-only-on-success discipline for the layout tree. Zero TODO/FIXME markers anywhere in the
  reducer stack.
- The frontend pane-state reducer family's clean non-overlap — no sibling reducer imports another's
  `reducer.ts`, coordination happens one layer up via explicit per-slice dispatch. `agent-pane-state/reducer.ts`
  specifically: injectable time, 145 tests, exhaustive unions, PR-provenance comments.
- `agentmux-launcher`'s `enforce_host_only` gate (one reusable authorization check called from every
  host-report arm) and `claim_terminal` (one atomic-ownership primitive shared by four termination paths) —
  both genuinely good centralization, worth citing as counter-examples to the copy-paste found elsewhere in
  the same file.
- `agentmux-cef`'s purity boundary is real, not aspirational — verified zero `.await`/blocking calls/IPC
  imports anywhere in `reducer/` except the one flagged violation (§2.2).
- The three-reducer architecture itself (srv/launcher/cef) is the *right* call, confirmed independently by
  the researcher assigned to check it: CEF tracks real OS/FFI handles that structurally cannot exist in srv's
  process; this is a legitimate process-boundary split, not a duplication smell.

---

## Appendix: research method

Six parallel, read-only research passes, each independently citing file:line for every claim: (1) `agentmux-
srv`'s Phase E reducer, (2) `agentmux-launcher`'s reducer + saga coordinator, (3) `agentmux-cef`'s own
reducer, (4) the frontend pane-state reducer family, (5) the full auth/credential state-machine cluster
(frontend and backend), (6) a focused check on whether the subagent-lifecycle "no reducer" claim from an
earlier report was still current. None of the six were shown each other's output before submitting; the
convergent finding — the same "shape shared, code not shared" pattern independently identified in the Rust
trio (pass 1-3) and the auth cluster (pass 5) — was not suggested by the prompts, which asked each pass to
characterize its own slice on its own terms. Cross-checked against `docs/reports/
REPORT_LARGE_FILE_MODULARIZATION_SCAN_2026_07_22.md` and `docs/specs/PLAN_SRV_REDUCER_MODULARIZATION_2026-05-
07.md` (both read in full) to avoid re-answering a question those reports already settled, and to confirm the
three-reducer architecture's origin was a deliberate 2026-05-07 decision rather than organic drift.
