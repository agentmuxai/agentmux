# PLAN: Splash telemetry — consolidated open-items tracker

**Date:** 2026-07-22
**Purpose:** Single place to track every still-open item across the splash-telemetry
work, replacing the scattered state where open items lived in three different
documents with no shared tally. Supersedes the "Stack status" section of
`SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_TELEMETRY_2026_07_02.md` (§ below) as the
source of truth for what's left — that section undercounted (named 3 open items;
a full read of its own body names 7).

**Sources consolidated:**
1. `docs/specs/SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_TELEMETRY_2026_07_02.md` — original design spec, 7 open items buried across §A.4/§B.4/§B.7.
2. `docs/analysis/ANALYSIS_SPLASH_SCREEN_TIMING_2026_07_20.md` — the count-up race, found and deferred the same day as PR #2244.
3. `docs/retro/retro-splash-screen-rows-dont-count-up-2026-07-22.md` — retro explaining why #2244 didn't fix the count-up bug.

---

## Status at a glance

| # | Item | ROI | Effort | Status |
|---|---|---|---|---|
| 1 | **Count-up race** — rows whose Begin+End land in the same tick snap instead of animating | **High** | Low | ✅ **Fixed this pass** |
| 2 | **B.7-1** — verify whether `cef`/`frontend`/channel-pruner stage events were ever wired | High (cheap) | ~0 | ✅ **Verified this pass — not wired** |
| 3 | **B.7-2** — verify Linux's shipped `SUMMARY_HOLD_MS`/hold-equivalent value | High (cheap) | ~0 | ✅ **Verified this pass — see finding** |
| 4 | Pre-supervisor stage instrumentation (macOS+Linux via `unix.rs`) | Medium | Low | ✅ **Done this pass** (Windows still open — see below) |
| 5 | §B.4.3 — consolidate `StageRow`/`apply_event` across Windows/Linux/macOS into one shared module | Medium (maintainability) | Medium | Open |
| 6 | A.4.4 — IPC-signal-based splash dismiss instead of file-existence poll | Low | Medium | Open (deferred by design) |
| 7 | Extend the `"host"` stage to cover full spawn→first-paint (currently spawn-latency only) | Medium | Medium-High | Open (blocked on #6's signal work) |
| 8 | A.4.1 — full srv/host spawn parallelization | Low (right now) | High | Open (deferred by design — real architectural dependency) |

Row-frozen-forever edge case (a row whose End never arrives before `ready_at` stays stuck in the "running" color forever) is a **related but distinct** defect from #1 — not fixed this pass, tracked separately below.

---

## 1. Count-up race — ✅ fixed this pass

**Root cause** (confirmed via code read + the 2026-07-20 analysis): `splash_mac.rs`'s render loop drained *all* pending `StartupEvent`s from the channel before redrawing. If a step's `Begin` and `End` were both already queued by the time a tick drained (i.e. the real work finished in well under one ~8–24ms tick), they were applied back-to-back in the same pass — the row was created already-`done`, so no "running" frame was ever painted. Visually: fast steps snap straight to their final value; only steps slower than one tick visibly count up.

**Fix:** `agentmux-launcher/src/splash_mac.rs` — extracted the per-tick event-application logic into a pure `apply_tick()` function. It now holds back (defers to the next tick) any `End` whose matching `Begin` was *also* seen in the same drain batch, guaranteeing every row spends at least one tick — and one render — in the "running" state before flipping to done. Deferred events from the previous tick are flushed first, before draining anything fresh, so genuinely slow steps (Begin and End in different ticks, the common case) are never held back an extra tick on top of their real duration.

Covered by 5 new unit tests (`splash_mac::tests::*`) exercising: same-tick Begin+End deferral (stage and sub-item), the deferred End applying cleanly next tick, the common different-tick case never being deferred, and a deferred-plus-fresh-same-tick-pair combination not cross-contaminating each other's state. `cargo test -p agentmux-launcher splash_mac`: 11/11 passing (6 pre-existing + 5 new). `cargo check`/`cargo clippy` clean on the touched code.

**Not fixed by this change** (separate defect, same root area): a row whose `End` genuinely never arrives before `ready_at` fires still freezes forever in the "running" (blue) color — that's a missing-event problem, not a same-tick-ordering problem, and needs its own fix (e.g. a fallback that marks any still-`None` row "unknown" once the hold starts, rather than leaving it visually implying it's still in flight). Tracked here as a follow-up, not scoped into this pass.

---

## 2. B.7-1 — cef/frontend/channel-pruner stage wiring — ✅ verified, not wired

Exhaustive grep across `agentmux-cef/src/**`, `agentmux-launcher/src/ipc/**`, and `frontend/**` for `report_startup_stage_*`/`ReportStartupStage*`/`channel.pruner`/`ChannelPruner`:

- The only two stages ever reported over this IPC path are **`"dlopen"`** and **`"cef_init"`** (`agentmux-cef/src/lib.rs:577-578, 884, 953`) — both CEF sub-stages of what the spec calls the `"host"` stage.
- **No** distinct top-level `"cef"` or `"frontend"` stage exists.
- **No** channel-pruner stage exists anywhere in the current tree — `SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md`'s concept, if implemented at all, was never wired into splash telemetry.
- `frontend/` has zero references to startup-stage reporting of any kind.

**Conclusion:** confirmed a real, still-open gap (item #7/"host" stage below), not a documentation error — anyone building on top of "the splash shows frontend-load timing" would be wrong to assume that.

---

## 3. B.7-2 — Linux's shipped hold-duration value — ✅ verified

`agentmux-launcher/src/splash_linux/mod.rs:224-229` (`min_hold()`): default is **450ms**, overridable via `AGENTMUX_SPLASH_HOLD_MS` — not the spec's cited "design intent" of 1500ms.

More importantly, this also confirms a **semantic mismatch** the spec didn't flag: Linux's `min_hold()` is a *minimum total on-screen time* (so a sub-perceptible flash doesn't happen on a very fast cold start) — it can overlap with real loading time. macOS's `hold_duration` (`splash_mac.rs`, default 2000ms) is a *pause added after* `ready_at`, specifically so the user can read the finished stage timeline before it fades. These are two different mechanisms answering two different questions, both currently reusing the same env var name (`AGENTMUX_SPLASH_HOLD_MS`) with different defaults and different effective behavior. Not fixed here — flagged as a real (if minor) design inconsistency worth a decision (either genuinely unify the semantics, or rename one to stop implying they're the same knob).

---

## 4. Pre-supervisor stage instrumentation — ✅ done for macOS+Linux, still open for Windows

**Correction to the original spec's framing:** its claim was "only Linux creates its `StartupEventSink` early enough in `main()`." Re-verified against current code and that's stale — macOS has created its sink in `main()` at the same point as Linux (before the splash exists) since the original macOS splash consumer landed (#1933). The **actual** gap, confirmed by grep across every real (non-selftest) call site of `stage_begin`/`stage_end`/`sub_begin`/`sub_end`, was different: only three stages were ever reported anywhere — `"migrations"`, `"backend"` (both `srv_spawner.rs`), and `"host"` (`unix.rs`/`windows.rs`). The pre-supervisor work in `run_unix` — path resolution, IPC socket setup, the single-instance handshake, the older-running-instances check, event-log/saga-registry setup — was never wrapped in any stage at all, on any platform. (The selftest fixture's synthetic `"prep"`/`"Launcher setup"` row in `main.rs` implied this stage already existed for real; it never did.)

**Fixed this pass, macOS+Linux (`agentmux-launcher/src/supervisor/unix.rs`):** moved the `startup_sink` resolution to the top of `run_unix` and wrapped everything from function entry through the single-instance handshake / event-log / saga-registry setup in a new `"prep"` stage, ending right before the `tokio::join!` that overlaps saga-coordinator setup with srv spawn. `cargo check`/`cargo test -p agentmux-launcher` both clean (222/222 passing) — purely additive, same `stage_begin`/`stage_end` idiom already used by the `"host"` stage, no control-flow changes.

**Windows still genuinely open, for a different and larger reason than "sink timing":** `run_windows` creates its *own* `startup_sink`/spawns its *own* splash internally (`supervisor/windows.rs:259,274`), well after its own pre-supervisor work (path resolution, Job Object creation) already ran — unlike macOS/Linux, Windows' splash doesn't exist yet at that point, so there's no live splash to report a "prep" stage to at all. Closing this gap needs restructuring `main.rs`'s Windows branch to create the splash earlier (mirroring the macOS/Linux pattern), which is a real, Windows-specific behavior change I can't build or verify from this (macOS) environment — correctly left open rather than attempted blind.

## 5. §B.4.3 — StageRow/apply_event consolidation — open

Windows (`splash.rs`), Linux (`splash_linux/mod.rs`), and macOS (`splash_mac.rs`, this file) each independently reimplement the same `StageRow`/`apply_event`/formatting state machine. Real cleanup opportunity, explicitly deferred when macOS's version was first added (2026-07-03) specifically because doing the extraction then would have meant editing two already-working splashes with no way to build/verify Windows from this (macOS) environment. Same constraint still applies today — still correctly deferred, not attempted this pass.

## 6. A.4.4 — IPC-signal-based dismiss instead of file-poll — open, deferred by design

`splash_mac.rs` polls `std::fs::metadata` every ~8ms for up to `DISMISS_TIMEOUT` (10s) waiting for the ready-file. Real, but minor CPU/wake overhead — not a latency win. Fixing it means replacing a dismiss protocol shared across all three platform backends and `agentmux-cef`'s `on_load_end` with an IPC-socket signal — real cross-crate risk for low payoff. Correctly still deferred.

## 7. Extend `"host"` stage to full spawn→first-paint — open, blocked on #6

Currently only spawn *latency* is instrumented (`agentmux-launcher/src/supervisor/{unix,windows}.rs`), not the full spawn-to-first-paint window. Needs the same race-safe first-paint signal work as item #6 — bundling them is the efficient path if #6 is ever picked up, since implementing one without the other leaves half the protocol change undone.

## 8. A.4.1 — full srv/host spawn parallelization — open, deferred by design

Blocked by a real dependency: the host's env vars (`ws_endpoint`, etc.) are parsed out of srv's self-reported `AGENTMUXSRV-ESTART` line, since srv self-binds a dynamic port. Two viable fixes exist (launcher pre-allocates the port, or host bootstraps via IPC-push instead of env vars) but both are materially bigger changes needing their own design spec and a full GUI-driven multi-instance test pass to verify safely. Correctly still deferred — highest raw upside of anything on this list, but not attempted here given the verification requirement this environment can't satisfy end-to-end.

---

## Why items 5-8 (and Windows' half of item 4) weren't attempted this pass

This pass worked through the highest-ROI items in order: #1 is the actual user-facing bug that prompted this consolidation; #2-3 were free (pure verification, no code risk); #4's macOS+Linux half was low-effort and used an already-proven idiom, so it got picked up too once the real gap was understood precisely. Items 5-8, and Windows' half of #4, are each real, but every one of them either (a) needs Windows-specific verification this single-platform (macOS) environment can't provide safely, or (b) was already deliberately deferred with sound reasoning that still holds. Picking one of them up should be a separate, individually-scoped pass — not bundled reactively into this one, per the same discipline the original spec called for and which this consolidation exists to make easier to follow going forward.
