# App API automation surface — tab / pane / window primitives

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-09
**Driving incident:** Phase 1 perf baseline retro (`docs/retro/perf-baseline-2026-05-09.md`) — wanted to drive 10× tab switches programmatically; the only path was windows-mcp pixel clicks, which had already false-negatived 11/24 trials in the pane-focus-stress harness. The user's call: "if you think you need windows-mcp, simply write the app API facility."
**Predecessors:** [`docs/specs/SPEC_HOST_API_AND_WIN32_PROBES.md`](SPEC_HOST_API_AND_WIN32_PROBES.md) (read-only test probes; this spec covers write primitives), `agentmux-srv/src/server/app_api.rs` (existing surface).

## Principle

**Every measurement, smoke-test, or repro need becomes an App API endpoint — never a pixel click.** Pixel-driving is fragile (DPI drift, layout edits invalidate coords, the harness already false-negatived 11/24 trials), opaque (a click might land on the chrome, the title bar, or nothing), and slow (typing the spec for a click sequence into windows-mcp is harder than coding the API call). The App API is the **only correct surface** for app-internal automation.

windows-mcp stays useful for things outside the app (taskbar clicks, foreground switches, cross-app coordination). For everything inside AgentMux, the answer is: extend the App API.

## Existing App API coverage (as of v0.33.738)

| Domain | Endpoints | Gap |
|---|---|---|
| Agent | `agent.open`, `agent.send`, `agent.stop`, `agent.status`, `agent.list`, `agent.output`, `agent.process-list`, `agent.tracked-blocks`, `agent.kill-process`, `agent.kill-tree` | — |
| Pane | `pane.open` | **No `resize`, no `split`, no `close`, no `list`, no `focus`** |
| Tab | (none in App API; `WorkspaceService.CreateTab` / `SetActiveTab` / `CloseTab` exist as low-level RPCs) | **No `tab.switch`, `tab.create`, `tab.close`, `tab.list`, `tab.tear_off`** |
| Window | (none) | **No `window.list`, `window.create`, `window.tear_off`** |
| Session | `session:archive`, `session:restore` | — |

## Proposed additions

### `tab.*` — tab manipulation

| Endpoint | Body | Returns | Notes |
|---|---|---|---|
| `tab.list` | `{ workspace_id?: string }` (defaults to active workspace) | `{ ok, tabs: [{ id, name, color?, blockids: string[], pinned: boolean }], active_id: string }` | Read; cheap; useful for the harness to discover tabs without parsing layout. |
| `tab.create` | `{ workspace_id?, name?, color?, copy_layout_from?: string }` | `{ ok, tab_id }` | Wraps `WorkspaceService.CreateTab`. `copy_layout_from` clones a layout for repro setup. |
| `tab.switch` | `{ workspace_id?, tab_id }` | `{ ok }` | Wraps `WorkspaceService.SetActiveTab`. **The driver of the perf baseline blocker.** Frontend reactive chain still fans out as-if a real click — the only difference is the source. |
| `tab.close` | `{ workspace_id?, tab_id, force?: bool }` | `{ ok }` | `force` skips the "are you sure" guard for unsaved blocks. |
| `tab.tear_off` | `{ workspace_id, tab_id, x?: number, y?: number }` | `{ ok, new_workspace_id, new_window_label }` | **The user's explicit ask.** Wraps `WorkspaceService.TearOffTab` + the existing `openTearOffWindow` flow but bypasses the OS-drag handshake (the `tear_off_sc_move_handshake` path is for human drag; programmatic tear-off jumps straight to creating the window at `(x, y)` or default position). |

### `pane.*` — pane manipulation

| Endpoint | Body | Returns | Notes |
|---|---|---|---|
| `pane.list` | `{ tab_id? }` (defaults to active tab) | `{ ok, panes: [{ block_id, view, geometry: {x,y,w,h}, parent_node_id }] }` | Read; informs the harness about geometry without `getBoundingClientRect`. |
| `pane.split` | `{ source_block_id, view, direction: "row"\|"column", position: "before"\|"after", url?, file?, agent_id? }` | `{ ok, new_block_id }` | Wraps `createBlockSplitHorizontally` / `createBlockSplitVertically`. Subsumes `pane.open` for the common "split off this existing pane" case. |
| `pane.close` | `{ block_id, force?: bool }` | `{ ok }` | Closes the block + collapses the layout node. |
| `pane.focus` | `{ block_id }` | `{ ok }` | Wraps `refocusNode`. Equivalent to a click without the IPC roundtrip. |
| `pane.resize` | `{ resize_handle_id, delta_px: number }` OR `{ block_id, target_size_px: number }` | `{ ok, applied_size }` | Drives the splitter programmatically. The first form mimics a drag tick; the second sets an absolute target. **Drives the perf baseline pane-resize measurement.** |
| `pane.tear_off` | `{ block_id, x?, y? }` | `{ ok, new_workspace_id, new_window_label }` | Wraps `WorkspaceService.TearOffBlock`; same bypass-the-OS-drag pattern as `tab.tear_off`. |

### `window.*` — window enumeration / creation

| Endpoint | Body | Returns | Notes |
|---|---|---|---|
| `window.list` | `{}` | `{ ok, windows: [{ window_label, workspace_id, active_tab_id, geometry, hwnd? }] }` | Read; covers the multi-window pool-drift diagnosis case from the prior session. `hwnd` is host-only. |
| `window.create` | `{ workspace_id?, x?, y?, width?, height? }` | `{ ok, window_label }` | Programmatic open-new-window. Used by harness setups. |
| `window.focus` | `{ window_label }` | `{ ok }` | OS-foreground the named window. The win32 layer (per `SPEC_HOST_API_AND_WIN32_PROBES.md`) has lower-level focus probes; this is the App-API-level "make it the active window" wrapper. |
| `window.close` | `{ window_label, save_session?: bool }` | `{ ok }` | |

## Threat model

Same as existing App API: token-gated WebSocket connection (`/agentmux/service` route, auth via `authenticate` or the dev-mode `authkey.dev` file). All write endpoints require the same authenticated session as `agent.send`. No new escalation surface — these endpoints expose primitives that already exist via lower-level RPCs (`WorkspaceService.SetActiveTab`, `TearOffTab`, `CreateBlock`); the App API just shapes them for **intent-bearing automation**.

`tab.tear_off` and `pane.tear_off` are programmatic equivalents of a user drag, which is already user-actionable. `window.create` is a programmatic equivalent of "new window from the menu". Threat class is identical: same-user local process, which we don't defend against today.

## Idempotency + return shape conventions

- All write endpoints return `{ ok: true, ... }` on success and `{ ok: false, error: string }` on failure. Never throw — match the existing `agent.send` / `pane.open` contract.
- `tab.switch` to the already-active tab is a no-op success (matches the existing `WorkspaceService.SetActiveTab` semantics; the frontend's `handleSelect` early-exits when `tabId === activeTabId()`).
- `tab.tear_off` / `pane.tear_off` to a new workspace are atomic — either the source moves and the new workspace exists, or the source is unchanged. (Existing `WorkspaceService.TearOff*` calls already enforce this; the App API wrappers should not weaken the guarantee.)
- `pane.resize` to an out-of-bounds size (below `MinNodeSizePx`) returns `{ ok: false, error: "size below minimum" }` and leaves the layout unchanged.

## Cross-references

- `agentmux-srv/src/server/app_api.rs` — current App API source. New endpoints land alongside.
- `frontend/app/store/global.ts` — `setActiveTab`, `createBlockSplitHorizontally`, `createBlockSplitVertically`, `refocusNode` — the frontend implementations the App API will marshal into.
- `frontend/app/drag/CrossWindowDragMonitor.win32.tsx::performTearOff` — the existing tear-off flow; the programmatic API mirrors steps 1-3 (TearOffTab/Block + openTearOffWindow) and skips step 4 (OS-drag handshake).
- `tools/tests/pane-focus-stress.ps1` — first consumer; switches from pixel-driving to App-API calls.
- [`SPEC_HOST_API_AND_WIN32_PROBES.md`](SPEC_HOST_API_AND_WIN32_PROBES.md) — read-only probes; complements this spec's write primitives.
- `docs/retro/perf-baseline-2026-05-09.md` — the perf retro that surfaced the gap (tab switch needed for N≥10 trials; no programmatic API today).
- Memory `feedback_user_drives_ui_for_baseline.md` — the broader principle this spec encodes.

## Effort estimate

| Phase | Endpoints | LOC | Days |
|---|---|---|---|
| Tab primitives | `tab.list`, `tab.create`, `tab.switch`, `tab.close`, `tab.tear_off` | ~300 | 0.5 |
| Pane primitives | `pane.list`, `pane.split`, `pane.close`, `pane.focus`, `pane.resize`, `pane.tear_off` | ~400 | 1 |
| Window primitives | `window.list`, `window.create`, `window.focus`, `window.close` | ~250 | 0.5 |
| Harness migration | swap pane-focus-stress + perf baseline driver to App API | ~150 | 0.25 |
| **Total** | 14 endpoints | ~1100 | **~2.25 days** |

Single PR per phase. Spec rides with the first phase (Tab) per `feedback_no_doc_only_prs`.

## Out of scope

- **A scripting layer / DSL on top.** The point of these endpoints is **lowest-level deterministic primitives** that test harnesses build on. Higher-level "open a 3-pane layout with two browsers and a terminal" is a sequence of these primitives, scripted in PowerShell or whatever the harness uses, not an API call.
- **Drag simulation.** `pane.resize` is the right level; if a future test needs to simulate the *trajectory* of a drag (for animation testing), that's a separate feature.
- **Recording / replay.** Out of scope here; if needed later, sits on top of these primitives + the win32 send_keys probe from the prior spec.
