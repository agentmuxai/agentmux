# Phase B roadmap (canonical, post-#595)

**Status:** Active reference. Updated 2026-04-28 after PR #595 (B.6 — single-instance mutex) merged. B.5 + B.6 complete; B.7 (frontend cutover) is next.
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
              ✓ B.6 — per-data-dir mutex single-instance (#595)
                        │
                        ▼
              ✓ B.7.1 — entries-bearing window-instances-changed (#596)
                        ▼
              ✓ B.7.2 — re-emit on BackendWindowId events (#597)
                        ▼
              ✓ B.9 — WRR observation + pure-reducer self-heal (#600)
                        ▼
              ✓ B.9.3 — pool-refill drain + cefsimple-pattern quit (#601)
                        ▼
              ✓ B.7.3.1 — launcher events to renderer via CEF JS bridge (#602)
                        ▼
              ◄── HERE. B.7.3.2 / B.7.3.3 / B.8 remaining for Phase B exit.
              B.7.3.2 ── prefer typed events for atom feeding;
                         demote `window-instances-changed` to fallback
              B.7.3.3 ── retire `window-instances-changed` + 4 sync emit sites
              B.8     ── Phase B exit (delete obsolete defensive code,
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

### B.6 — single-instance mutex (done — PRs #595 + #598 + #599 + direct-to-main 2ffa63c5)

- Launcher synchronously binds `first_pipe_instance(true)` BEFORE spawning srv/host. The pipe bind is the AUTHORITATIVE single-instance signal.
- On `ERROR_ACCESS_DENIED`, the second launcher reads `<launcher-shared-data-dir>/ipc-port` (written by the host post-CEF-init) and forwards an `open_new_window` HTTP POST to the existing instance, then exits 0. Mirrors the status-bar version popup's "new window" UX.
- Three iterations to land it:
  - **#598**: initial forward implementation; transient classification.
  - **2ffa63c5** (direct-to-main, mistake — should have been a PR): port-file path was at the cef cache dir but launcher reads at the launcher-shared data dir.
  - **#599**: read response after POST so the launcher's process exit doesn't tear down the TCP connection before axum's async handler runs.
- The MessageBox path is reserved for genuine bind failures (namespace misconfig). Stale-state defect (gap #8) is bounded because pipe-bind happens first: a stale port file is irrelevant on the first-instance path (overwritten); on the second-instance path the live first instance wrote a fresh port:token, so forwarding lands.
- Smoke verified on v0.33.482: second `agentmux.exe` launch yields one new window in the existing instance, no dialog. Launcher log + host log both confirm.

### B.7 — frontend cutover (3 PRs)

- B.7.1 (#596): replaced `app-init.ts::refreshLabels(retriesLeft)` polling with launcher-driven re-emit carrying resolved entries.
- B.7.2 (#597): re-emit on `BackendWindowIdRegistered/Unregistered` so windowId `null → real` transitions update the InstancePanel without a follow-up RPC.
- **B.7.3.1 (done — PR #602)**: host outbound CEF JS bridge `launcher_event_bridge.rs` forwards every typed `Event` to every top-level renderer via `Frame::ExecuteJavaScript` calling `window.__agentmux_launcher_event(<json>)`. Renderer-side `frontend/util/launcher-events.ts` registers the dispatcher into a SolidJS signal pair; `frontend/app/store/launcher-event-reducer.ts` runs `createEffect` over it. B.7.3.1 logs only — bespoke `window-instances-changed` still feeds atoms.
- B.7.3.2 (next): promote typed events to authoritative; demote `window-instances-changed` to fallback. Atom-mutating handlers replace the bespoke listener body.
- B.7.3.3: retire bespoke channel + 4 sync emit sites in `commands::window`, `drag`, `window_pool`, `client.rs`.

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
| Single-instance lives in launcher pipe bind, not host port-file | B.6 (#595) | Pipe handle is OS-owned → no stale-state path; ERROR_ACCESS_DENIED is the canonical second-instance signal; user-facing MessageBox replaces silent exit |
| WRR uses event-driven Win32 hooks, no timers / heartbeats | B.9 (#600) | `SetWinEventHook` + `WM_WINDOWPOSCHANGED` deliver every needed transition synchronously; the reducer fires drift on the same dispatch tick |
| Pool refill is suppressed during host drain via `is_quitting` flag | B.9.3 (#601) | CEF's `quit_message_loop` is QuitWhenIdle — without suppressing refill, pool windows keep state.browsers non-empty forever and idle is never reached |
| Cross-thread "deliver work to UI thread" uses `cef::post_task` (or Win32 PostMessage as bypass) — NOT direct calls or PostThreadMessage | B.9.3 (#601) | Direct calls from worker thread are CEF UB; PostThreadMessage(WM_QUIT) is ignored by CEF's custom Windows pump; `cef::post_task` is the documented portable bridge — and Win32 `PostMessage(hwnd, WM_CLOSE)` bypasses CEF's task queue when needed |

## How to update this doc

1. After every Phase B PR merges: tick the box in the migration table; update "Where we are."
2. After a major direction change (like the multi-reducer decision): add a row to the decisions log.
3. Don't open a PR for changes here — local working notes only.
4. Future agents resuming work should read this first; if you're confused after a context compression, the answer is here.
