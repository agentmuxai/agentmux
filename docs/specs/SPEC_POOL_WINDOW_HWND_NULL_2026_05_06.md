# Pool window HWND-null at promote time

**Date:** 2026-05-06
**Owner:** AgentA
**Status:** spec
**Related:** [`SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md`](./SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md) (PR #704), [`SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`](./SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md)

---

## 1. Problem

Pool windows are spawned correctly, mark `renderer ready`, get enqueued — but their underlying Win32 HWND is null when `promote_pool_window` tries to use them. Every cross-window tear-off observed during PR #704 smoke testing fell back to the cold path because the host's pool reported `host window handle is null (state inconsistency)`.

Trace from v0.33.648 / `agenta/tear-off-pool-path` smoke (3 tear-offs in a row, all reproduced):

```
06:38:56.718  [pool] promoting pool window   label=window-pool-813cdce63a...
06:38:56.718  ERROR  host window handle is null (state inconsistency)
06:38:56.718  [pool] orphan close_browser issued — on_before_close will run cleanup + refill
06:38:56.718  WARN   [pool] pool exhausted on tear-off — frontend will cold-path
06:38:56.724  WARN   [pool] pool window destroyed before promote — releasing semaphore + refilling
```

The pool slot was created at `06:27:32` and consumed at `06:38:56` — 11 minutes idle. Subsequent attempts at 06:39:03 and 06:39:26 all hit the same error against fresher pool slots (sub-second to ~1-min idle).

## 2. Why it matters

- The frontend correctly tries the pool first (PR #704); the pool's failure mode is what breaks the user-visible behavior. Result: every tear-off shows the cold-path's 150–300ms first-paint flash that the pool was specifically designed to eliminate.
- The cold-path code path is also where the source-side renderer crash occasionally fires (per `SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md` §1 finding (B)). Fixing this spec eliminates both the flash AND the destabilization risk in one go.
- Pool refill machinery wastes work spawning replacements that will themselves go HWND-null and be discarded.

## 3. Investigation surface

The host's `promote_pool_window` is the place that detects the failure:

- `agentmux-cef/src/commands/window_pool.rs::promote_pool_window` — pops a label from `state.pool.queue`, looks up the Browser, fetches its `BrowserHost::window_handle()`. If null → emits `ERROR host window handle is null (state inconsistency)`, dispatches `close_browser`, releases the semaphore, returns `None`.
- `agentmux-cef/src/commands/window_pool.rs::mark_pool_window_renderer_ready` — fires the `pool window renderer ready, enqueued` log. Called from a frontend signal that the renderer has fully initialized.

The mismatch: a pool window is "renderer ready" but has no HWND. Possible causes (none yet confirmed):

1. **Pool windows are CEF-views browsers, not Win32 HWNDs.** When `cef::window_create_top_level` returns, the wrapper has a CefViews handle but the underlying HWND may not be assigned until later (or possibly never for hidden windows). `BrowserHost::window_handle()` returns the platform handle, which for a hidden CefViews window may be null until the window is shown.
2. **HWND was destroyed by the OS while the window was off-screen.** Pool windows are positioned off-screen at `(-32000, -32000)` per `[ipc] WRR-POS hwnd=... rect=(-32000,-32000)-(...)` log lines. Windows occasionally cleans up off-screen windows that never receive input. Unlikely but possible.
3. **Race with a CEF lifecycle callback.** The renderer-ready signal may fire from a worker thread before the UI thread has fully attached the HWND. If frontend dispatches a tear-off in the gap, the host sees `Some(browser)` from the queue but its HWND lookup races with the UI thread's HWND assignment.

The 11-minute-idle case argues against (3) (way too long for a thread race) but supports (1) or (2).

## 4. Diagnosis (2026-05-06)

Instrumented diagnostic build (PR #704 follow-up) captured this:

```
07:14:17  HWND probe at register time: valid  hwnd=0x130892
07:15:39  HWND probe at renderer-ready time   cef_status=null  cached_status=cached=0x130892 alive=true
```

The HWND is **alive** per Win32 `IsWindow`, but `BrowserHost::window_handle()` returns null. Pattern:

- **Register time:** CEF's `BrowserHost::window_handle()` returns the valid HWND.
- **Renderer-ready time** (~80s later in dev mode, ~1-2s in production): CEF returns null. The cached HWND is still a live OS window.

So this is **none of (1), (2), or (3)** as previously hypothesized. CEF Views' `BrowserHost::window_handle()` is unreliable after the page loads — the underlying Win32 window is intact, CEF just stops reporting its handle. Root cause likely lives inside CEF's view-attachment lifecycle (the parent HWND association on the view changes once content is fully loaded).

### 4.1 Fix: cache the HWND at register time

Pool windows have one moment when CEF reliably reports their HWND: `on_after_created`, where `register_pool_window` already runs. Cache the HWND there in a module-level `Mutex<HashMap<String, usize>>`. At promote time, fall back to the cache if `host.window_handle()` returns null. Verify cache is still a live OS window via `IsWindow` before using; if not (genuine OS-level destruction), refuse the promote and let the frontend cold-path.

Cache cleanup happens in `on_pool_window_destroyed`, which fires for any `window-pool-*` close (including post-promote user-window closes that retain the prefix).

### 4.2 Why not just always use the cached HWND

`BrowserHost::window_handle()` is preferred when it returns non-null — that's the path CEF intends, and a future CEF version may stop returning null after page-load. Keeping the CEF call as the first try means the moment CEF stabilizes, no further code change is needed.

## 5. Out of scope

- Refactoring the pool to not rely on Win32 handles at all. The whole point of the warm pool is to preserve the underlying CEF browser; promote-time HWND access is necessary for SC_MOVE handshake (Phase 4 of `SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`).
- Cross-platform behavior. macOS / Linux pool implementations have their own HWND-equivalent (NSWindow / X11 Window); same diagnosis-first approach should apply, but the actual cause may differ.
- Reducing pool size. The pool is fine sized at 2; the bug is independent of how many slots are in flight.

## 6. Tests

Hard to unit-test because the bug lives at the CEF/Win32 boundary. Options:

1. **Manual smoke.** Open AgentMux → wait 30s → tear off a tab. Confirm pool was used (BrowserRegistered with `is_pool: true`). Repeat for 5min idle, 30min idle.
2. **Property-style instrumentation.** Add a metric `pool.promote_hwnd_null_count` and emit it in launcher logs. After this fix lands, expect zero increments under normal operation. Regression would be obvious.

## 7. References

- `agentmux-cef/src/commands/window_pool.rs` — pool spawn / enqueue / promote
- `agentmux-cef/src/commands/drag.rs:308` — `tear_off_pool_promote` host handler
- `frontend/app/drag/tear-off-pool-helper.ts` — frontend's try-pool-first wiring (PR #704)
- v0.33.648 task dev smoke trace — `~/.agentmux/dev/agenta-tear-off-pool-path/logs/`
