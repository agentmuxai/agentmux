# Phase F + G Roadmap (post-architecture-completeness)

**Date:** 2026-05-01
**Author:** AgentA
**Status:** Synthesis after the architecture-completeness sequence shipped (steps 1-7).
**Supersedes:**
- `multi-reducer-proposal-2026-04-28.md` for what to *do* about Phase F+G (the proposal itself stays as the *vision* doc).
- `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §13 (Phase F preview) — already superseded by `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`; this doc supersedes the F-spec's §9 sub-PR sequence in light of what shipped.
- `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §14 (Phase G preview) for the *plan*; the section's high-level shape stays accurate.

**Reads-this-first:**
- `next-steps-architecture-completeness-2026-05-01.md` — the 7-step plan we just executed.
- `reducer-architecture-gaps-2026-05-01.md` — the gap inventory; this doc updates it with what got closed and what stayed open.
- `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` — the F-spec; partially shipped.
- `multi-reducer-proposal-2026-04-28.md` — original vision (still load-bearing).

---

## 0. Why this doc

Two phases were sketched but never fully designed: **Phase F** (host reducer) and **Phase G** (event-sourced, drop SQLite). The architecture-completeness sequence shipped *parts* of Phase F under the same naming, while leaving Phase G entirely untouched. The original Phase F spec listed 7 sub-PRs (F.1 through F.7); we shipped some directly, folded others into other phases, and discovered the original sequencing didn't survive contact with reality.

This doc:
1. Reconciles what we *actually* shipped against the F-spec's plan.
2. Captures **what we learned** during the 7-step sprint that changes the F+G picture.
3. Refines the F-remainder plan to match current state.
4. Re-evaluates the Phase G go/no-go decision factors now that Phase E + partial F are real, not hypothetical.

---

## 1. Where we are (snapshot)

### What shipped from Phase F (per the original F-spec §9 numbering)

| F-spec PR | Original scope | What actually shipped | Status |
|---|---|---|---|
| **F.1** | Host reducer skeleton | ✓ #629 — skeleton + `pending_window_creations` arm in one PR (collapsed F.1+F.2). | **MERGED** |
| **F.2** | `pending_window_creations` arm | ✓ Folded into F.1 (#629). | **MERGED** (in F.1) |
| **F.3** | Drag arms migration | ❌ Not shipped. **Deferred to Chrome-faithful tear-off spec Phases 2-7** — the user explicitly chose to skip rather than migrate code that the tear-off rewrite will replace. |
| **F.4** | Tear-off hook arms | ❌ Not shipped. Same reason — fold into tear-off spec Phase 2 work. |
| **F.5** | Pool-respawn saga | ✓ #634 — cross-process dispatch deferred (`IssueCmd::Host` is logged-only because the launcher→host pipe doesn't exist yet; saga relies on host's existing implicit `spawn_pool_window`). | **MERGED** with documented limitation |
| **F.6** | Window-cleanup cascade | ❌ Not shipped. |
| **F.7** | Cleanup audit + proptests | ❌ Not shipped (E.7-equivalent on host reducer). |

### What shipped from outside the original F-spec scope

| What | PR | Why this counts toward F+G readiness |
|---|---|---|
| **E.6** renderer multi-source dispatcher + saga buffering | #630 | Required infrastructure for any cross-process saga to be visible to the renderer. F.5 depended on this. |
| **E.4 Option A** layout reducer focused/magnified arms | #632 | Closes part of `handle_move_tab` migration tolerance (the strict-mode flip is still pending follow-up). The full layout migration (Option B) is still deferred. |
| **Saga durability PR 1** (schema + SagaLog API + SagaCtx instrumentation) | #631 | Persist subscriber for sagas; foundation for crash-recovery. Phase G's event-sourced model can reuse this infrastructure verbatim for events. |
| **Step 5: DeleteBlock + DeleteTab sagas** | #633 | Converted SQLite-first delete patterns to proper sagas. Closes a major Phase E `merge_meta_patch`-style compromise. DeleteWorkspace still wcore-direct. |
| **Step 7: E.7 integration tests** | #635 (in flight) | End-to-end saga tests + cross-pipe ordering + recovery-from-crash scaffold. Phase E's exit gate. |

### What's still open from `reducer-architecture-gaps-2026-05-01.md`

Updated gap status (was 7 categories; now ~5):

1. **§1 Phase E sub-phases.** E.4 *Option A* shipped; *Option B* (full layout — rootnode/leaforder migration) deferred. E.6 done. E.7 in flight (#635).
2. **§2 Phase F implementation.** F.1, F.5 partial. F.3, F.4, F.6, F.7 still open. F.2 folded.
3. **§3 robustness gaps (saga durability, pool-promote, renderer registration, per-Event saga_id).** Saga durability PR 1 shipped; PR 2 (resume-on-restart + `--diag sagas`) pending. Cross-process pool-promote shipped *as a saga but not as a real cross-process dispatch*. Renderer registration as saga step still open. Per-Event saga_id correlation still deferred (codex flagged twice during F.5; mitigated with evict-and-replace serialization).
4. **§4 reducer-pattern compromises.** `handle_move_tab` migration tolerance still in place pending E.4 Option A soak. SQLite-first deletes: Block + Tab now sagas (#633); Workspace still wcore-direct (deferred follow-up). `merge_meta_patch` pass-through unchanged.
5. **§5 cross-pipe coordination.** Per-source version tracking + saga buffering shipped (E.6). Force-push protocol still informal. Snapshot-replay-before-live-events still informal.
6. **§6 platform parity.** Unchanged (Windows-only `--diag`).
7. **§7 Phase G.** Still long-term ceiling; this doc refines the call.

---

## 2. What we learned (changes the F+G picture)

Five lessons from the 7-step sprint that the original F-spec didn't anticipate.

### 2.1 Bot reviewers oscillate on fundamental design tensions

PR #633 (delete sagas) ran 5 rounds of review with reagent and codex flipping positions on the last-tab guard:
- Round 1: saga pre-check has TOCTOU race (reagent P1).
- Round 2: reducer guard breaks `Cmd+W` semantics (codex P1).
- Round 3: walk back the guard; codex re-flags TOCTOU (codex P2).
- Round 4: same.
- Round 5: settle with `force: bool` flag — atomic guard with explicit bypass for compensation paths.

**Implication for F+G:** any operation with multiple legitimate caller intents (user-driven vs. compensation vs. cascade) needs an explicit modality parameter, not a one-size-fits-all rule. The `force: bool` pattern from #633 is reusable — bake it into the next round of reducer arms.

### 2.2 Cross-process sagas without cross-process dispatch are still useful

F.5 shipped with `IssueCmd::Host` as a log-only no-op (no launcher→host pipe yet). Codex initially objected; we kept it because the renderer-visible value (`SagaStarted` / `SagaCompleted` brackets) is preserved either way — the host's existing implicit refill produces the matching `Event::PoolWindowAdded`. This pattern is **the saga-as-narrator pattern** — the saga doesn't drive the work, it observes and brackets it.

**Implication for F+G:** F.6 (window-cleanup cascade) can ship the same way. The narrator pattern is acceptable as a checkpoint until cross-process dispatch lands. F.6 doesn't need to wait for the launcher→host pipe.

### 2.3 Concurrent same-kind sagas need explicit serialization

F.5 round 1 broadcast events to all in-flight sagas, mis-correlating concurrent promotes (codex P1). Round 3 silent-dropped overlapping triggers; round 4 (final) evicts and replaces. **Conclusion: until per-event `saga_id` correlation lands, every saga kind needs an explicit policy** for "what happens when two of me are in flight simultaneously":
- **Drop-overlap** (F.5 round 3) — works only if the saga's terminal condition is guaranteed to fire eventually.
- **Evict-and-replace** (F.5 round 4) — works for sagas where a stalled prior is correctly abandoned.
- **FIFO queue** — works for sagas where ordering matters and triggers should never be lost.

This decision becomes a per-saga concern. F.6 will need to make the same call.

### 2.4 Bot oscillation breaks the merge rule's heuristic

The "merge if reagent APPROVED + codex no fresh P1/P2" rule breaks down past 3 rounds because each round produces new findings. We added a *meta-rule* — "if bots oscillate ≥3 rounds, document the limitation and merge anyway" — which the user signed off on for #633 round 5 + F.5 round 4. **For F+G remainder, expect at least one oscillation per non-trivial PR.** Plan rounds 4-5 into estimates, not just round 1-2.

### 2.5 The `merge_meta_patch` pass-through stays valuable

The original gaps doc §4 listed `merge_meta_patch` as a compromise to close. After watching delete sagas run 5 rounds for atomic-guard-vs-compensation, **rebuilding meta updates with field-level reducer commands would multiply that oscillation by every meta field times every entity**. Recommendation: leave `merge_meta_patch` as a deliberate escape hatch. Document that opaque-meta is an explicit affordance, not an oversight.

---

## 3. Phase F remainder — refined plan

Drops / consolidates per the lessons in §2 and what shipped in §1.

| Step | Scope | Status post-architecture-completeness | Estimate |
|---|---|---|---|
| **F.3 (deferred)** | Drag arms migration. | Folded into Chrome-faithful tear-off spec Phase 2-4. **Don't migrate F.3 separately** — wasted work since tear-off rewrite replaces the triad anyway. | N/A |
| **F.4 (deferred)** | Tear-off hook arms. | Folded into tear-off spec Phase 2. Same logic as F.3. | N/A |
| **F.6** | Window-cleanup cascade saga. | **Ready when there's appetite.** Apply the saga-as-narrator pattern from F.5: launch the saga as a bracketing observer; `IssueCmd::Host` is log-only until cross-process pipe lands. Pick an explicit serialization policy per §2.3. | ~250 LOC |
| **F.7** | Host reducer property tests + cleanup audit. | Mirrors E.7. After F.6 lands. | ~400 LOC |
| **F.5 (post-merge follow-up)** | Cross-process dispatch (launcher→host pipe + replace `IssueCmd::Host` log-only with real send). | Distinct PR sequence; not a Phase F continuation. **Treat as its own mini-phase.** Estimate: ~600-800 LOC across 2-3 PRs. Spec needed. |
| **Saga durability PR 2** | Resume-on-startup + `--diag sagas` + crash-recovery integration tests. | Already speced in `SPEC_SAGA_DURABILITY_2026-05-01.md` §9. Independent of F.6/F.7. | ~400 LOC |
| **Step 5 follow-up** | DeleteWorkspace cascade saga. | Speced in `next-steps-architecture-completeness-2026-05-01.md` step 5 PR 2. ~250 LOC. |
| **Step 3 follow-up** | E.4 Option B (rootnode + leaforder + pendingbackendactions through reducer with node-path representation). | **Don't ship until something demands it.** Speculative complexity per `SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md` §3. |
| **`handle_move_tab` strict-mode flip** | Reinstate workspace_id check + drop lazy-import. | Speced in `next-steps-architecture-completeness-2026-05-01.md` step 3 PR 2. Wait for E.4 Option A to soak. ~100 LOC. |

**Refined Phase F = ~1500 LOC across ~6 PRs**, not the original ~2100 LOC across 7 PRs. The reduction is from F.3+F.4 deferred to tear-off spec.

**Sequencing:** F.6 before F.7. Saga durability PR 2 in parallel (independent). Cross-process dispatch as its own follow-up. Tear-off spec Phase 2 work folds in F.3+F.4 organically.

---

## 4. Phase G — go/no-go reconsidered

Phase G's original sketch (§14 of `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md`):

- G.1 snapshot writer in srv
- G.2 bootstrap: snapshot+replay (no SQLite)
- G.3 one-time SQLite→event-log migration
- G.4 retire `WaveStore` for migrated types
- G.5 log truncation policy

**Bottom line of the original:** "Phase G is the right architecture if the reducer pattern works. Phase E is the validation that it works. Make the call after E ships, not before."

Phase E shipped. Time to make the call.

### 4.1 What Phase E + partial F validated about the reducer pattern

✓ **Reducer events as the cross-reducer contract works.** The E.6 multi-source dispatcher consumes srv + launcher events cleanly. Bots routinely caught state-divergence bugs that came from breaking the discipline (`merge_meta_patch` opaque, layout heal_layout direct write, etc.). The pattern produces tractable bugs — they're real, but they're *findable*.

✓ **Saga coordinator works for in-process flows.** Phase E.5 shipped 4 sagas; #633 added 2 more; F.5 added 1 cross-process saga (with documented limitation). All have been observable, debuggable, and crash-resistant via the durable saga log (#631).

✓ **Persist subscriber works as a single-direction projection.** wstore writes happen exclusively in the subscriber. No direct mutations leaked into reducer dispatch paths once the deletes converted (#633).

⚠ **What didn't validate cleanly:** cross-process dispatch is still vapor (F.5 dispatch is log-only). Per-event `saga_id` correlation came up twice and got punted twice. These don't disprove the pattern, but they show the architecture has *unfinished* pieces, not just *unstarted* ones.

### 4.2 Phase G's cost-benefit is now measurable

| Phase G benefit | Realized after E+F? |
|---|---|
| Single source of truth | **Yes** for srv reducer state — the persist subscriber's projection is a write-only tail. SQLite divergence has been the source of zero merged-PR bugs in E+F. |
| No persist-subscriber | Worth ~1500 LoC of code deletion. Real maintenance saving. |
| Schema evolution via event versions | Marginal — we haven't done a SQLite migration in months. Event versioning would need its own discipline anyway. |
| Cleaner mental model | **Yes** — but this is the benefit hardest to quantify. |

| Phase G cost | Realistic now |
|---|---|
| Snapshot writer + correctness | Bigger than the original sketch suggests. The reducer state we'd snapshot includes per-saga state, per-source counters, in-flight sagas. Snapshot consistency under concurrent dispatch is non-trivial. |
| One-time migration | Manageable. The event log is already durable; we'd emit synthetic events from SQLite once at startup. |
| Log truncation policy | Real complexity. Need bounds on log size, snapshot interval, retention policy. |
| External consumers of `wstore.db` | Need inventory. Some tools (debug, post-mortem analysis) probably read the DB. |

### 4.3 Recommendation

**Defer Phase G further. Specifically: don't start G until both:**

1. **F.6 + F.7 ship + soak for one minor version.** The host reducer needs to be solid before doubling down on the pattern. Currently F is 2/6 PRs complete; not enough validation surface.
2. **Cross-process dispatch lands** (the launcher→host pipe). Phase G's mental model assumes events flow uniformly; if half the system still relies on synchronous in-process calls, the snapshot model has integrity holes.

When both gates clear, G becomes a focused 5-PR sequence as originally sketched. No re-spec needed — `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §14 is still accurate.

**What Phase G is NOT a good idea for:** retrofitting it onto a partially-complete F. The migration would inherit every pending compromise (drag arms still wcore-direct, tear-off hooks still module-level, etc.) and force them into the snapshot model.

### 4.4 If we never ship Phase G

That's also fine. The reducer pattern + persist subscriber is the dominant architecture; SQLite as a projection is a finite cost. The benefits of dropping it are real but not urgent. Phase G is the architectural ceiling, not a load-bearing requirement.

If F.6 + F.7 + cross-process dispatch ship and the team's appetite for foundational refactors has cooled, Phase G can stay parked indefinitely. The current architecture is *correct*, just not *minimal*.

---

## 5. Sequencing recommendation

Three independent threads, prioritized:

### Thread 1: Finish Phase F core
1. **F.6** window-cleanup cascade saga (saga-as-narrator pattern; ~250 LOC).
2. **Saga durability PR 2** resume-on-startup + `--diag sagas` (independent; ~400 LOC).
3. **F.7** host reducer property tests (after F.6; ~400 LOC).

### Thread 2: Step 5 follow-ups (cleanup compromises)
1. **DeleteWorkspace saga** (~250 LOC).
2. **`handle_move_tab` strict-mode flip** (~100 LOC; wait for E.4 Option A soak).

### Thread 3: Cross-process dispatch (its own mini-phase)
1. **Launcher→host command pipe** (~400 LOC of IPC plumbing + back-pressure).
2. **F.5 / F.6 dispatch swap** — replace `IssueCmd` log-only with real send (~200-400 LOC).
3. **Per-event `saga_id` correlation** (proper FIFO routing) (~300 LOC).

After all three threads complete, **re-evaluate Phase G**. Don't pre-commit.

---

## 6. What this doc does NOT do

- **Doesn't write the F.6 spec.** Sketch in F-spec §7.2 + lessons in §2 are enough to scope it.
- **Doesn't write the cross-process dispatch spec.** That's a real spec because the IPC design is non-trivial.
- **Doesn't supersede `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`** — that spec stays load-bearing for F.6 + F.7 design. This doc just refines its sequencing.
- **Doesn't change Phase G's design.** Original §14 sketch is still accurate; only the timing changes.
- **Doesn't address tear-off spec Phases 2-7.** Separate effort that *consumes* F.3+F.4 territory.

---

## 7. Cross-references

- `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` — host reducer spec; partially shipped.
- `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §14 — Phase G sketch; still accurate.
- `SPEC_SAGA_DURABILITY_2026-05-01.md` — saga log spec; PR 1 shipped, PR 2 pending.
- `SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md` — E.4 Option A shipped; Option B deferred.
- `next-steps-architecture-completeness-2026-05-01.md` — the 7-step plan we just executed.
- `reducer-architecture-gaps-2026-05-01.md` — original gap inventory; this doc updates the open-gap list.
- `multi-reducer-proposal-2026-04-28.md` — vision (still load-bearing).
- `phase-e-status-2026-05-01.md` — Phase E status.
- `SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md` — Chrome-faithful tear-off; consumes F.3+F.4 territory.

---

## 8. History

- **2026-05-01** initial draft, after the architecture-completeness 7-step sprint shipped (steps 1-6 merged; step 7 in flight).
