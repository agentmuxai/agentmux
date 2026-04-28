# Phase B roadmap (canonical, post-#594)

**Status:** Active reference. Updated 2026-04-28 after PR #594 (B.5 finish — scaffolding-role audit) merged. B.5 is complete; B.6 (single-instance mutex) is in flight.
**Author:** AgentA.
**Read first if resuming Phase B work**, then `b5-migration-architecture-2026-04-28.md` and `multi-reducer-proposal-2026-04-28.md`.

---

## Where we are

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
                        │     ✓ window_instance_registry (#579-#584)
                        │     ✓ window_id_map (#585-#589)
                        │     ✓ window_meta (#590-#592, sync-cache refinement)
                        │     ✓ B.5 finish — scaffolding-role audit (#594)
                        │     deferred: browsers, pool maps (Phase F — see
                        │       multi-reducer-proposal-2026-04-28.md)
                        ▼
              B.5 complete. 3 of 5 maps fully migrated; 2 deferred to
              Phase F with explicit scaffolding comments in code.
                        │
                        ▼
              ◄── HERE. B.6 in flight: single-instance mutex.
              B.6  ──  per-data-dir mutex single-instance (named pipe already covers it)
              B.7  ──  frontend cutover (delete polling)
              B.8  ──  Phase B exit (delete obsolete defensive code,
                       add property tests, --diag tool, CI smoke)
                        │
                        ▼
                  Phase B done — golden vision (intermediate form)
                        │
                        │  Phase D, E, F — see multi-reducer-proposal
                        ▼
                  Multi-reducer architecture (long-term destination)
```

## Migration table (B.5)

| Map | step a | step b | step c | step d | step e | Notes |
|---|---|---|---|---|---|---|
| `window_instance_registry` | ✓ #579 | ✓ #580 | ✓ #581 + #582 | ✓ #583 | ✓ #584 | Pure data, fully retired |
| `window_id_map` | ✓ #585 | ✓ #586 | ✓ #587 | ✓ #588 | ✓ #589 | Pure data, fully retired |
| `window_meta` | ✓ via B.4 | ✓ #590 | ✓ #591 | ✓ #592 | n/a | Sync-cache exception — host_meta stays |
| `browsers` | n/a | n/a | n/a | n/a | n/a | **Deferred to Phase F** (FFI handles) |
| `window_pool` + `unpromoted_pool_labels` | partial via B.4 | partial via B.4 | n/a | n/a | n/a | **Deferred to Phase F** (sync lifecycle scaffolding) |

See `b5-migration-architecture-2026-04-28.md` for why `browsers` and pool maps can't follow the standard ratchet, and `multi-reducer-proposal-2026-04-28.md` for the long-term plan.

## What's left for Phase B

### B.5 finish (done — PR #594)

- Scaffolding-role comments added to `state.browsers`, `window_pool`, `unpromoted_pool_labels`, and `compute_and_report_host_counts` so future agents see why these fields don't follow the standard ratchet and where they head in Phase F.

### B.6 — single-instance mutex (in flight)

- The named-pipe `first_pipe_instance(true)` already rejects a second launcher binding the same pipe. Just need to surface a clear error when launch fails ("AgentMux is already running, PID N") and delete any old port-file probe.

### B.7 — frontend cutover (2-3 PRs)

- Replace `app-init.ts::refreshLabels(retriesLeft)` polling with subscription to launcher events via the CEF JS bridge.
- Wire host's outbound JS bridge to forward launcher events to the renderer.
- Frontend reducer fed by the event stream.

### B.8 — Phase B exit (1-2 PRs)

- Property tests for invariants from `ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md`.
- `agentmux.exe --diag` Tool client that prints launcher state.
- CI synthetic close-all + assertion.
- Delete obsolete defensive code (e.g., host-side `app-init.ts` retries that polling drove).

## Beyond Phase B

| Phase | Scope | Why deferrable |
|---|---|---|
| **Phase D** | `GetSnapshot` resync, `--diag` Tool, persisted event log | Foundations laid (versioned events, monotonic counters); just need snapshot RPC + ring-buffer + replay |
| **Phase E** | srv state machine for tabs/panes/layout | Independent of B; same reducer pattern applied to srv. **First validation point for multi-reducer** |
| **Phase F** | Host state machine — retire scaffolding model | After E validates multi-reducer infrastructure, retrofit host. Migrates `browsers` + pool maps into host-reducer state |

## Decisions log

(Don't relitigate these.)

| Decision | When | Rationale |
|---|---|---|
| Tokio runtime in launcher | B.2 design | Standard, srv already uses it |
| No reducer state persistence (memory only) | B.3 design | Spec default; workspaces persist via srv DB |
| Frontend ↔ launcher via host JS bridge | B.7 design | Renderers stay sandboxed; host is trust boundary |
| Migrate incrementally (sub-PR sequence) | B plan | Bugs are usually edge cases not architecture; migration keeps app running |
| Codex hallucinates < gemini hallucinates | empirical, PRs #573-592 | Gemini auto-review disabled; reagent + codex is the merge gate |
| `WindowInstanceRegistry` migrates first | B.5 plan | Smallest map, simplest semantics |
| Window-count drift only on window-level transitions | B.4b round-2 | Pool transitions can fire mid-flight; pool-only check via `ReportHostPoolCount` |
| `window_meta` keeps sync cache (not full delete) | B.5 step d round-2 (codex P1) | `open_subwindow` parent check + cascade-close need synchronous local state |
| `browsers` + pool maps deferred to Phase F | 2026-04-28 | FFI handles + sync lifecycle scaffolding can't migrate via standard ratchet |
| Multi-reducer is the long-term architecture | 2026-04-28 | Cleaner than "scaffolding outside the model"; deferred to Phase E + F to validate the pattern incrementally |
| `docs/retro/*.md` files are local-only | 2026-04-28 | No review churn; future agents read them via `MEMORY.md` pointer |

## How to update this doc

1. After every Phase B PR merges: tick the box in the migration table; update "Where we are."
2. After a major direction change (like the multi-reducer decision): add a row to the decisions log.
3. Don't open a PR for changes here — local working notes only.
4. Future agents resuming work should read this first; if you're confused after a context compression, the answer is here.
