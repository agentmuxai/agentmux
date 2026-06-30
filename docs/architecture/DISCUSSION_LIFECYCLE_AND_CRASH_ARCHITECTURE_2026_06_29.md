# Discussion / Decision Record — Lifecycle & Crash Architecture

**Date:** 2026-06-29
**Type:** Decision record (captures a working discussion; not yet a committed plan)
**Status:** Direction agreed in discussion; pillars not yet scheduled
**Owner:** asaf

> Purpose: pin down the architecture-level conclusions reached in discussion so the two
> intertwined threads — **lifecycle/teardown** and **crashes/OOM** — don't get lost or
> re-litigated. This is the index that ties together the specs written today and the existing
> tracking issues. Read `SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR_2026_06_29.md` for the full audit.

---

## 1. The core realization (one root, two threads)

The lifecycle/teardown churn and the crash/OOM churn are **the same defect seen from two angles:**

> **The CEF host is both the component most likely to die (Chromium aborts on any failed
> allocation, `0xE0000008`) and the authoritative in-memory owner of session/UI/lifecycle state.
> Fragile + irreplaceable = every death is catastrophic and every recovery is hand-built.**

That single inversion is why we keep shipping mitigations on both threads (gated recovery, memory
supervisor, crash budgets, pause pages, graceful-exit on the crash side; orphan reconciliation,
quit-decision rewrites, saga compensation on the lifecycle side).

## 2. The decided direction — disposability symmetry

Three-tier authority model:

```
srv       = the ONE durable authority (SQLite). Source of truth. Survives everything.
host      = projects srv's logical topology into native OS objects (windows/panes/pool). DISPOSABLE.
renderer  = projects srv's state into UI (SolidJS frontend).                              DISPOSABLE.
```

**Rule:** host and renderer each hold only a *projection* of srv's truth — never the truth itself.
If either dies, a fresh instance reprojects from srv.

### Where we are vs. where we're going
- **Renderer — already disposable.** Its state is a projection of srv; on crash, gated recovery
  reprojects. This is why renderer death is already survivable.
- **Host — NOT disposable today.** It owns pane/window/pool/drag/lifecycle state in volatile Rust
  memory. Host death loses the truth → catastrophe.
- **Goal:** make the host disposable the same way the renderer is — the asymmetry *is* the defect.

### Clarifications captured from the discussion (so they're not re-debated)
- **"Disposable," not literally "stateless."** The host always holds native OS handles (HWNDs, CEF
  browser objects, GPU process, window pool) that *can't* live in srv. srv owns the **logical**
  topology (which windows/panes exist, layout, agent bindings); the host rebuilds the native objects
  from that on reproject. The renderer is closer to truly stateless (owns no native resources).
- **A reproject is just an involuntary restart.** The host already rebuilds all windows from srv on a
  normal cold launch. "Make host death survivable" = "make the existing startup-restore path good
  enough to fire unplanned, mid-session."
- **Flicker is a crash-path concern only — NOT normal operation.** Steady-state use never rebuilds
  windows, so nothing flickers. Reconstruction is visible only on the reproject (crash/restart) event.
- **The steady-state tax is invisible write-through, not UX.** Making srv authoritative means host
  state mutations (pane move, window open, focus) persist to srv. Must be async/batched so it never
  stalls interaction. srv already persists layout, so this extends an existing path.

## 3. What this collapses (why it's a refactor, not more patches)
- Host OOM → reproject instead of catastrophe → kills the crash-catastrophe class.
- Nothing unsaved to compensate → the ~4,000-line **saga durability layer collapses** to an
  in-memory registry.
- "Is state durable or do I flush on close?" incoherence disappears → always "durable in srv,
  reproject." Removes the contract that forced `orphan_reconcile` into existence.
- Overcommit prevented at the source → the supervisor ladder / crash budgets / magic floors collapse.

## 4. The two threads, mapped to existing tracking

### Thread A — Lifecycle / teardown
- **Root:** quit decision fragmented across `on_before_close` (edge), WRR `maybe_quit_on_last_user_window`,
  `orphan_reconcile.rs`; the principled fix `reducer/quit.rs::reconcile_quit` is **written, tested, and
  `#[allow(dead_code)]` — NOT WIRED.**
- Related issues: **#768** (host/frontend lifecycle divergence), **#1681** (floating-pane DnD lifecycle
  rethink), **#1461** (redock state-reducer/native-HWND lifecycle), **#1662** (pool redock empty spot),
  **#864** (retire wcore-direct layout path — the layout split-brain).
- Related docs: `retro-lifecycle-teardown-churn-2026-06-22.md`,
  `SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md`.

### Thread B — Crashes / OOM
- **Root:** Chromium aborts on OOM; host is the stateful victim; no proactive admission control.
- Related issues: **#942** (Service Supervision & Recovery umbrella), **#376** (renderer death + host
  hang), **#778** (GPU crash → black panes).
- Related docs (today): `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md`,
  `SPEC_GRACEFUL_OOM_EXIT_2026_06_29.md`. Prior: `SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md`,
  `SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`, `INCIDENT_2026_06_26_APP_CLOSED.md`,
  `SPEC_MEMORY_ANALYSIS_2026_06_26.md`, `retro-oom-crash-2026-06-16.md`.

## 5. The trend that justifies the rethink (from the historical audit)
- ~45–55 of every 100 PRs touch memory/lifecycle/crash; rate **flat-to-rising over ~1,000 PRs**.
- ~**1 in 5 commits is a follow-up or revert.**
- Renderer-OOM recovery re-fixed **6+ times**; replaceChild **4×** (last named `FULL_ANALYSIS_AND_FIX`);
  cross-channel continuity **15** commits with a *"fixed then broke again"* retro.
- 29+ dated memory/lifecycle/crash docs, continuous **2026-03-26 → today**, densest in June.

## 6. Decisions
- **D1.** Adopt the three-tier disposability model (srv authority; host & renderer disposable projections).
- **D2.** Treat lifecycle and crash as one program of work, not two patch streams.
- **D3.** Pause net-new OOM/lifecycle band-aids beyond cheap P0 wins until the pillars are sequenced.
  (The `SPEC_GRACEFUL_OOM_EXIT` work shrinks substantially once host death is a reproject — ship only
  its P0 "suppress the ugly OS box + native dialog" now.)

## 7. Open questions (to resolve before scheduling Pillar 1)
- **Q1.** What exactly is the host's authoritative state set that must move to srv? (pane topology,
  window→pane mapping, pool warm-state, focus/magnify, drag intent) — enumerate it.
- **Q2.** Reproject UX bar: acceptable reconstruction time and visual treatment on crash-restore?
- **Q3.** Write-through mechanism: event-sourced through the reducer (ties into **#864**) vs. snapshot?
- **Q4.** Can admission control (Pillar 3) ship independently and early, before the host rework? (Likely yes.)

## 8. Next steps — see §"Best next steps" in the closing discussion / `SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR` §6.
Sequence: **Pillar 2 (wire `reconcile_quit`)** → **Pillar 3 (admission control)** → **Pillar 1 (host
reproject)** → saga collapse + persistence pay-down (#864, agents Phase 3b/3c). Add the missing E2E
test ("close last window ⇒ tree exits", "host OOM ⇒ session reprojects").
