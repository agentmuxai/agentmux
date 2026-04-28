# Phase B roadmap to the golden vision

**Status:** Active reference. Updated 2026-04-28 after PR #582 (B.5c orphan cleanup) merged.
**Author:** AgentA.
**Companion docs:**
* `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` — driving spec
* `specs/ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md` — pre-migration inventory of host's 13 state stores
* `docs/retro/phase-b-plan-2026-04-28.md` — original sub-PR sequence (B.1–B.8) with risk analysis
* `docs/retro/migration-pattern.md` — the a/b/c/d/e ratchet pattern per state field
* `docs/retro/audit-vestigial-types-2026-04-28.md` — pre-Phase-E cleanup audit

## Golden vision

The launcher owns canonical OS-level state via a pure reducer:

```
update(state, command, ctx) → Vec<Event>
```

* **Commands** flow client → launcher.
* **Events** flow launcher → clients (broadcast).
* **State** lives only in the launcher.
* Other processes (host, srv, frontend, tools) hold *projections* of state that they maintain by consuming events.

Bugs from "host's `window_meta` and `window_instance_registry` fell out of sync" become structurally impossible: one state, one mutator, one place to enforce invariants.

## Where we are (2026-04-28)

```
Pre-Phase-B  ──► host owns 13 HashMaps  ◄── started here
                        │
                        │  B.1 (#570/571/572)  ──  srv as launcher-spawned sibling
                        │  B.2 (#573)          ──  named-pipe IPC server
                        │  B.3 (#574)          ──  pure reducer skeleton
                        ▼
              Foundation laid: launcher has Tokio + IPC + reducer ✓
                        │
                        │  B.4  (#576)  ──  window mirror (read-only)
                        │  B.4a (#577)  ──  pool tracking
                        │  B.4b (#578)  ──  drift detection
                        ▼
              Mirror tracks reality, drift observable ✓
                        │
                        │  B.5 (a→b→c→d→e per map, smallest first)
                        │     window_instance_registry:
                        │       a (#579)  ──  launcher-authoritative registry
                        │       b (#580)  ──  host shadow + drift logs
                        │       c (#581)  ──  read-path cuts over to shadow
                        │       c-fix (#582) ── orphan cleanup, surfaced by drift
                        ▼
              ◄── HERE (after PR #592 merged 2026-04-28).
                  ✓ window_instance_registry — fully migrated (a→e)
                  ✓ window_id_map — fully migrated (a→e)
                  ✓ window_meta — migrated with refinement (host_meta
                    kept as synchronous cache; see "window_meta
                    exception" below)
                  Remaining: `browsers` (full a→e), pool maps (c/d/e),
                  then B.6/B.7/B.8.
                        │
                        ▼
              B.6  ──  per-data-dir mutex single-instance
              B.7  ──  frontend cutover (delete polling)
              B.8  ──  Phase B exit (delete host state stores)
                        │
                        ▼
                  Phase B done — golden vision reached
```

## What's left

### B.5 — migrate the remaining maps (a/b/c/d/e per the migration-pattern doc)

| Map | step a | step b | step c | step d | step e |
|---|---|---|---|---|---|
| `window_instance_registry` | ✓ #579 | ✓ #580 | ✓ #581 + #582 | ✓ #583 | ✓ #584 |
| `window_id_map` | ✓ #585 | ✓ #586 | ✓ #587 | ✓ #588 | ✓ #589 |
| `window_meta` | ✓ baked into B.4 | ✓ #590 | ✓ #591 | ✓ #592 (refined — see below) | not applicable |
| `browsers` | — | — | — | — | — |
| `window_pool` + `unpromoted_pool_labels` | partial via B.4 | partial via B.4 | — | — | — |

### window_meta exception

The standard a→e ratchet calls for step e to fully delete the host's field. For `window_meta`, codex's PR #592 review caught two cases where the launcher-fed shadow alone is insufficient:

1. **task dev mode** — launcher IPC absent; shadow stays empty. `open_subwindow`'s parent-validation needs a synchronous local source.
2. **Cascade-close race** — child opens just before parent closes; launcher round-trip hasn't completed by the time `on_before_close` fires; `subwindow_children_of` would miss the child.

The refined design keeps `host.window_meta` as a **synchronous local cache**, written from a single canonical site (`on_after_created` from the popped `PendingWindowCreation` entry) and removed from the symmetric `on_before_close`. The launcher's `state.windows` remains canonical for cross-process queries; `window_meta` covers same-process synchronous lookups.

**Implication:** for any future map migration, before step e, check whether the field has any same-process synchronous lifecycle-checking consumer. If yes, that map likely follows the same cache-not-delete pattern.

**Pool maps note:** B.4 already added launcher's `state.pool` + host-fed reports. So pool migration starts at step c, not a. The combined map (`window_pool` queue + `unpromoted_pool_labels` set) is also conceptually one in the launcher (`State.pool: HashSet`), so the cutover collapses two host fields into one.

**Recommended order** (3 maps done, 2 remaining):
1. ~~Finish `window_instance_registry`~~ ✓ done.
2. ~~`window_id_map`~~ ✓ done.
3. ~~`window_meta`~~ ✓ done (with sync-cache refinement).
4. **`browsers`** — biggest. CEF `Browser` handles can't be sent over the wire, so this map's launcher representation is just labels + metadata; the actual `Browser` object stays in host as a non-reducer field. Likely follows the window_meta cache-not-delete pattern (same-process synchronous lookup needed).
5. Pool maps — c/d/e only (skips a/b since B.4 covered them).

**Estimated PRs**: ~14 more (4 remaining maps × ~3 steps avg + 2 for `window_instance_registry`'s d/e).

### B.6 — single-instance mutex

* Spec invariant: at most one launcher per data dir (multi-launcher races the named pipe + corrupts state).
* Pre-Phase-B used a TCP port-file probe. Now trivial: `ServerOptions::first_pipe_instance(true)` already rejects a second launcher binding the same pipe.
* Work: delete the old port-file check in srv/host, add a clear error message when the second launch fails to bind, surface to user as a dialog.
* **~1 PR, ~80 LoC.**

### B.7 — frontend cutover

* Currently: frontend polls `list_windows()` / `get_window_count()` via `app-init.ts::refreshLabels(true, retriesLeft)` — the retry loop exists because host's state was racy.
* B.5 makes host's state authoritative-via-launcher, but frontend still polls.
* B.7: wire the launcher's event stream to the frontend via CEF JS bridge. Frontend subscribes once, gets push updates instead of polling.
* Bonus: deletes `app-init.ts`'s ~50 LoC retry loop. Frontend becomes simpler, not more complex.
* **~2-3 PRs**: JS bridge plumbing (host emits to renderer process); frontend reducer + atom integration; delete polling loop.

### B.8 — Phase B exit (cleanup)

* Delete host-side state stores entirely. After B.5 step e for every map, host's `AppState` shrinks from 13 fields to ~3 (CEF browser handles, IPC port, etc. — non-reducer infrastructure).
* Property-based tests assert all 13 store-related invariants from the inventory analysis.
* `agentmux.exe --diag` (Tool client) prints canonical state + CI runs a synthetic close-all + assertion.
* Six core invariants from spec §7 checked at every transition; violations panic (Job Object reaps via OS).
* **~1-2 PRs.**

## Beyond Phase B

| Phase | Scope | Why now-easy-because-of-B |
|---|---|---|
| **C** | Warm pool consolidation — typed `WarmPool` separate from main registry, type system prevents mixing pool with full instances | After B, pool is already its own type in launcher; just delete the parallel host fields |
| **D** | `GetSnapshot` / resync protocol, persisted event log, `--diag` tool | Foundations laid: versioned events, monotonic counters, host-IPC pipe. Need to add the snapshot RPC + ring-buffer log + replay logic |
| **E** (proposed, not blocking B) | srv state machine for tabs/panes/layout | Independent of B — could happen in parallel; same reducer pattern applied to srv. Audit doc covered the prep |

## Decisions log (so future sessions don't relitigate)

| Decision | When | Reasoning |
|---|---|---|
| Tokio runtime in launcher (3 MB binary growth) | B.2 design | Standard, battle-tested, srv already uses it. Hand-rolled threads were 50 KB but ~5× more code. |
| No reducer state persistence (memory only) | B.3 design | Spec default. Workspaces/tabs persist via srv DB; window restoration is a separate UX feature, deferred. |
| Frontend ↔ launcher via host JS bridge (not direct pipe) | B.7 design | Renderers stay sandboxed. Host is trust boundary. ~1ms latency irrelevant for state events. |
| Migrate incrementally (9-PR sequence vs greenfield rewrite) | B plan | Bugs in this codebase are usually from edge cases, not architecture. Migration keeps app running while new model gets exercised. |
| Codex hallucinates < gemini hallucinates | empirical, PRs #573-582 | Merge gate: reagent + codex. Gemini auto-review disabled (#582 was the last straw — gemini misread the orphan-cleanup bug). On-demand `@gemini review` still works. |
| `WindowInstanceRegistry` migrates first in B.5 | B.5 plan | Smallest map, simplest semantics (just `HashMap<String, u32>` + monotonic counter). Validates the migration pattern on the easiest case. |
| Window-count drift detection only on window-level transitions | B.4b round-2 fix | Pool transitions can fire during in-flight close paths where window counts are mid-mutation; pool-only check (`ReportHostPoolCount`) preserves "check every transition" without false positives. |

## Tech debt + open items

| Item | Source | Plan |
|---|---|---|
| Cross-thread interleaving in drift detection produces transient false positives | codex P2 round-5 on PR #578, accepted as known limitation | Defer to B.5 cleanup or B.8; needs transition-ID protocol |
| Reagent silently drops on GitHub 504 | filed `a5af/reagent#114` | Wait for upstream fix |
| `docs/retro/b1-srv-spawner-design.md` + `phase-b-plan-2026-04-28.md` untracked | sitting locally for ~10 PRs | Land in a small docs-only commit when convenient |
| Smoke tests are manual | every PR cycle | Phase B exit criteria: synthetic close-all in CI |
| Codex round-5 P2 (transition IDs) | PR #578 | Above; same issue |

## Calendar estimate

At our recent pace (~3-6 hours of bot-review cycles per PR-merge cycle, ~1-3 PRs per session):

* **B.5 remaining maps**: ~3-5 working sessions
* **B.6**: 1 session
* **B.7**: 2-3 sessions (frontend territory; expect surprises)
* **B.8**: 1 session if everything before went smoothly

**Total to golden vision: ~7-10 working sessions.**

## Recommended next moves (post-PR #582)

1. **Smoke v0.33.463** when current build lands. Validates B.5c orphan-cleanup fix; specifically watch for:
   * `[pool] orphan close_browser issued` log line on failed promote
   * Zero sustained `DriftDetected { kind: Windows, ... }` events
   * Pool size stable at `POOL_TARGET_SIZE = 2` after failed promote
2. **B.5d for `window_instance_registry`** — drop host's eager register/unregister. Open design question: race on first frontend lookup. Two options:
   * Frontend retries on `unwrap_or(0)` lookup miss
   * Host pre-seeds shadow synchronously when calling `report_window_opened` (computes expected number locally — risky if launcher state diverges)
   Recommendation: option 1, simpler.
3. **B.5e** — delete `WindowInstanceRegistry` struct + rename `shadow_*` field. Almost pure delete.
4. Start the next map — `window_id_map`. Re-runs the a/b/c/d/e ratchet on a fresh field; if anything in the migration-pattern doc needs revision based on lessons from #579-582, update it then.

## How to use this doc in future sessions

1. **Read this first** before opening a new B.5 sub-PR. It tells you which step you're on and what's already done.
2. **Update the "Where we are" diagram + the B.5 migration table** when a PR merges.
3. **Add to "Decisions log"** when a non-obvious choice is made (so we don't relitigate).
4. **Add to "Tech debt"** when something is deferred — even if you think you'll come back to it (you might not).
