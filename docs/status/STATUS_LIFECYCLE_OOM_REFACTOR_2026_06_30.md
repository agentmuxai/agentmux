# Status — Lifecycle / OOM Refactor Program

**Date:** 2026-06-30
**Type:** Living status doc (snapshot of an in-progress program)
**Program:** Make the recurring Windows OOM crashes + lifecycle/teardown churn survivable by
inverting the architecture — srv as the durable authority, host & renderer as disposable
projections. See `SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR_2026_06_29.md` and
`DISCUSSION_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_06_29.md`.

---

## 1. Where we are (one-glance)

| Workstream | State |
|------------|-------|
| Architecture analysis + decision | ✅ Merged (#1847) |
| OOM crash diagnosis + graceful-exit design | ✅ Merged (specs in #1847) |
| **Pillar 2 — single lifecycle authority** | 🟡 Stage 1 merged (#1850); Stage 2 designed, blocked on runtime verification; Stage 3 pending |
| Pillar 3 — commit-aware admission control | ⬜ Specced (P0), not started |
| Pillar 1 — host reproject | ⬜ Deepest; deliberately last |
| Saga collapse + persistence paydown | ⬜ After Pillars 1–3 |
| Disk cleanup + tooling | ✅ Done (~107 GB reclaimed; `bin/clean-agentmux-builds.ps1`) |

---

## 2. Merged this program

- **#1847** — architecture health audit + refactor proposal, OOM specs (`SPEC_WIN10_PAGEFILE_OOM_CRASH`,
  `SPEC_GRACEFUL_OOM_EXIT`, `SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR`, `SPEC_PILLAR2_WIRE_RECONCILE_QUIT`),
  the discussion/decision record, and the PF status-chip fix. (Codex + reagent reviewed; corrected the
  `SEM_NOGPFAULTERRORBOX`-would-kill-WER-dumps issue both bots caught.)
- **#1850** — **Pillar 2 Stage 1**: `reconcile_quit` wired into the reducer as a pure, level-triggered
  decision. `DispatchOutput.request_drain` + `is_quit_relevant` negative guard; un-dead-coded the
  decision fns; 2 new tests; full reducer suite green (51 passed). **Behavior-neutral** — nothing
  consumes `request_drain` yet, so the legacy edge gate still drives the drain. Merged at `b7518a70`.

## 3. Pillar 2 — remaining stages

### Stage 2 (next in sequence) — the deadlock-sensitive core
Extract the `on_before_close` Stage-1 cascade into `Client::begin_drain_and_cascade(&self, reason)`,
consume `request_drain` at the UI-thread CEF callbacks, delete the inline count gate.

**Design resolved (in `SPEC_PILLAR2_WIRE_RECONCILE_QUIT` §3.2):** `host_dispatch` is `&self` on
`AppState` with no `Arc<Self>` handle, so consumption is **inline at the UI-thread callbacks**, not a
cross-thread post from `host_dispatch`.

**Correctness obligation (the hard part):** the drain must be re-evaluated at *every* transition that
can reach "drainable" — not just last-window close. Confirmed settling paths that occur on *other*
callbacks:
- last-window close → `on_before_close` (dispatches `UnregisterBrowser`)
- a pending **user** "New Window" resolving/aborting → `DequeuePendingWindowCreation` (different callback)
- pool spawn/promote/destroy settling after a refill race

Miss one → orphan race returns silently. Act inline on the wrong one (e.g. `quit_message_loop` from
`on_before_close`) → reintroduce the documented UI-thread deadlock.

**Verification gate:** Stage 2's correctness is a *runtime* property (no missed callback, no deadlock).
Unit tests proved the decision (Stage 1) but cannot prove the consumption wiring is complete or
deadlock-free. **Stage 2 must NOT auto-merge on bot approval** — it needs the app run + the E2E
"close last window ⇒ tree exits" test passing. This is why it's a separate, gated PR.

### Stage 3
Demote `orphan_reconcile` + WRR to pure executors (drop their independent drain decisions); add the E2E
test. With the host eventually stateless (Pillar 1), the ~4k-line saga layer can then collapse.

## 4. Pillar 3 — commit-aware admission control (parallel-able)
Specced P0 in `SPEC_WIN10_PAGEFILE_OOM_CRASH`. Gate agent-turn (`claude.exe`) spawn on available commit
*before* launching; queue when short; per-agent working-set cap. Independent of Pillar 2 and more
unit-testable (pure headroom check + queue, no CEF deadlock surface) — a good candidate to build in
parallel while Stage 2 awaits runtime verification. Re-derive the reserve from `PrivateUsage`, not
`VirtualMemorySize64` (the 6/26 analysis over-counted using VM size).

## 5. Merge/approval audit (2026-06-30)
Checked after a concern that work may have merged without approval. **All clear:**
- #1850 (Stage 1) merged by `a5af` after reagent APPROVED.
- #1828 (column-dissolve, earlier this session) merged by `AgentY-asaf` at 05:29 — *after* reagent
  APPROVED at 05:27 (following 4 resolved CHANGES_REQUESTED rounds).
- #1837 flagged by a batch scan but a direct query confirms reagent APPROVED before merge (API blip).
- No `pillar2`/`reconcile_quit`/`begin_drain` PR exists in any state — no unverified Stage 2 landed.

## 6. Loose ends
- Stale remote branches (merged, not auto-deleted): `docs/oom-lifecycle-architecture`,
  `feat/pillar2-wire-reconcile-quit` — safe to delete.
- **Mis-homed work:** the AgentFooter status-label code (`deriveStatusLabel`/tool-aware labels) is
  committed on `fix/macos-display-name-cfg-guards`, not on its own PR. Separately, another agent shipped
  a related "Working…" liveness thread (#1841 spec, #1842 watchdog) — reconcile before reviving the
  status-label work to avoid duplication. (Out of scope for the lifecycle/OOM program; parked.)

## 7. Recommended next step
Either:
- **(a)** Implement **Pillar 2 Stage 2** as a verification-gated PR (do-not-merge until E2E/runtime
  pass), or
- **(b)** Build **Pillar 3** (admission control) in parallel — completable + unit-testable end-to-end
  without the runtime gate.

Recommendation: **(b) Pillar 3** for an end-to-end completable increment now, with Stage 2 done next
alongside a live verification pass.
