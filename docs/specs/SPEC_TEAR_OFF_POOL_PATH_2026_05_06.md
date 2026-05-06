# Tab Tear-Off — Always Use Warm Pool + Source-Side Renderer Crash

**Date:** 2026-05-06
**Owner:** AgentA
**Status:** spec
**Related:** [`SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`](./SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md), [`SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md`](./SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md)

---

## 1. Problem

Two bugs surface together when the user tears a tab off into a new window:

**(A) Tear-off bypasses the warm pool.** The host has a Phase-6 pre-warmed pool (`tear_off_pool_promote` in `agentmux-cef/src/commands/drag.rs:308`) that holds 2 ready-to-go CEF windows so tear-off can promote one with no first-paint flash. The HTML5 drag path (`CrossWindowDragMonitor.{win32,darwin,linux}.tsx`) never calls `tearOffPoolPromote`; it goes straight to `openWindowAtPosition` (cold path, ~150–300ms flash). Only `tabbar.tsx`'s SC_MOVE handshake path tries the pool first.

**(B) Source window's renderer crashes ~3s after a cold-path tear-off.** Repro from v0.33.647 portable smoke test:

```
06:13:07.295  start_cross_drag        source_window=main
06:13:07.311  workspace.TearOffTab    OK
06:13:07.311  open_window_at_position label=window-dca56f9d... (COLD PATH)
06:13:07.321  BrowserRegistered       is_pool=false
06:13:08.87   destination booting
06:13:09.50   destination DragOverlay listening (alive)
06:13:10.428  ERROR renderer process crashed   ← source's renderer
              error_code=-36861 detail="Crashpad_NotConnectedToHandler"
```

The destination window is alive and well. The source's renderer dies. The host's `on_render_process_terminated` handler (per [`SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md`](./SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md)) catches it and shows the recovery dialog — that part works.

(A) is the proximate cause of (B): if the warm pool is used, the cold-path codepath that destabilizes the source isn't exercised. Fixing (A) likely papers over (B) for the common case, but (B) is its own bug worth tracking.

## 2. Why this matters

- **First-paint flash.** Cold path is 150–300ms of blank window before content paints. Pool promote is ~0ms.
- **Crash exposure.** Cold-path window creation is the same path that has the long-standing Phase-1 freeze investigation in `commands/mod.rs:create_isolated_request_context`. Fewer trips through that code = fewer crash opportunities.
- **Pool waste.** Pool windows are spawned eagerly on app start and refilled after use. If tear-off never consumes them, they sit idle and the refill machinery does nothing useful.
- **Drift signal.** v0.33.647 launcher log: `DriftDetected { kind: Pool, host_count: 0, mirror_count: 2 }` — the mirror tracks pool slots that the host's reducer claims aren't there. Aggravated by the cold-path code creating `window-{uuid}` labels that share the pool spawning machinery without going through the promote flow.

## 3. Root cause for (A)

`CrossWindowDragMonitor.{win32,darwin,linux}.tsx` predates the warm-pool work. Its `handleCrossDragEnd` function:

```ts
} else if (dragType === "tab" && payload.tabId) {
    const newWsId = await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
    if (newWsId) {
        await api.openWindowAtPosition(screenX, screenY, newWsId);
    }
}
```

Direct cold-path call. No pool attempt. Same shape for the `pane` branch and on macOS / Linux variants.

`tabbar.tsx::performTabTearOff` (lines 600–710) does the right thing: tries `tearOffPoolPromote` first, falls back to `openWindowAtPosition` only if that throws. But that path is only reached when the drag goes through the SC_MOVE handshake, not the legacy HTML5 dragend path.

## 4. Decision

### 4.1 (A) Wire tear-off pool promote into `CrossWindowDragMonitor`

Replace the direct `openWindowAtPosition` calls with the same try-pool-first pattern `tabbar.tsx` uses:

```ts
} else if (dragType === "tab" && payload.tabId) {
    const newWsId = await WorkspaceService.TearOffTab(payload.tabId, sourceWsId);
    if (newWsId) {
        try {
            await api.tearOffPoolPromote(newWsId, screenX, screenY);
        } catch (poolErr) {
            // Pool exhausted or unavailable — fall back to cold path.
            await api.openWindowAtPosition(screenX, screenY, newWsId);
        }
    }
}
```

Same change for the `pane` branch. Same change in `.darwin.tsx` and `.linux.tsx`.

The pool fallback is conservative — if `tearOffPoolPromote` throws (e.g., pool exhausted, host rejects for any reason), we cold-path the same way `tabbar.tsx` does. No regression risk.

### 4.2 (B) Source-side renderer crash investigation (separate scope)

Out of scope for this PR. To track:

- After cold-path tear-off, the SOURCE window's renderer crashes ~3s later.
- The destination window finishes initializing fine.
- After (A) lands, this should rarely fire (cold path becomes the exception). If it still reproduces with the pool path, it's a deeper tear-off-induced source-side bug.

Suspected causes worth investigating in a follow-up:

1. The frontend's tab-removal rerender in the source window triggers a state mutation that crashes the renderer (e.g., a race on `documentAtom` or layout tree).
2. The cross-window drag cleanup (`setCurrentDragPayload(null)`) racing with another listener.
3. The launcher's `HwndDriftDetected` event flooding the source window's launcher-event-bridge after the new window registers.

## 5. Out of scope

- Refactoring CrossWindowDragMonitor entirely (e.g., merging into `tabbar.tsx`'s SC_MOVE flow). The current design has both paths for OS-specific reasons; consolidation is a separate effort.
- The Phase-1 cold-path freeze investigation in `commands/mod.rs::create_isolated_request_context`.
- Pool sizing / refill policy.

## 6. Tests

`CrossWindowDragMonitor` doesn't currently have its own tests (the harness mocks `getApi()` at integration time). Adding focused tests requires either:

- A unit test that mocks `getApi()` and verifies `tearOffPoolPromote` is called before `openWindowAtPosition` for both tab and pane dragtypes.
- A manual smoke step in BUILD.md: tear off a tab, verify the new window's `BrowserRegistered` event has `is_pool: true` (confirming pool was used).

For this PR, the manual smoke is the validating signal. Add it to `docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`'s smoke section.

## 7. Implementation order

1. Update all three `CrossWindowDragMonitor.*.tsx` files (~10 lines each, identical change).
2. Bump patch version.
3. Build portable, smoke-test:
   - Open new tab → tear off → confirm log shows pool promote, not cold-path
   - Look for the BrowserRegistered event with `is_pool: true`
   - Confirm the source renderer doesn't crash (validates (B) is largely papered over by (A))
4. If source renderer still crashes occasionally, file a separate issue against (B).

## 8. References

- `agentmux-cef/src/commands/drag.rs:304-340` — `tear_off_pool_promote` host handler
- `agentmux-cef/src/commands/drag.rs:342-426` — `open_window_at_position` (cold path)
- `frontend/app/tab/tabbar.tsx:600-710` — reference implementation (try-pool-first)
- `frontend/app/drag/CrossWindowDragMonitor.win32.tsx:237,242` — current cold-path-only call sites
- `frontend/util/cef-api.ts:589-594` — `openWindowAtPosition` + `tearOffPoolPromote` IPC bindings
- `agentmux-cef/src/client/mod.rs:1089` — `on_render_process_terminated` recovery handler
- v0.33.647 portable smoke trace — `~/.agentmux/versions/0.33.647/logs/agentmux-host-v0.33.647.log.2026-05-06`
