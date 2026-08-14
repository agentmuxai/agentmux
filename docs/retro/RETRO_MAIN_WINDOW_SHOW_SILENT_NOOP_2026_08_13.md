# Retro: main window's `on_load_end` show() silently no-ops, leaving a blank/grey window

**Date:** 2026-08-13
**Severity:** High when it hits (app appears completely broken — blank grey
window, no error anywhere) but intermittent, not deterministic.

## What happened

Launching `task dev` on a shared multi-agent dev machine repeatedly (not
every time) produced a window that rendered as a totally blank grey
rectangle. Confirmed via `agentmux-host-*.log`:

- The only window-creation event ever logged was `PoolWindowAdded { label:
  "window-pool-<uuid>" }` — never `WindowOpened` for `"main"`.
- `mem_attribution` heartbeats showed `panes_mb="0"` indefinitely.
- The frontend logged `[initApp] pool mode — deferring init until
  pool:promote or pool:new-window` (`frontend/app/init/pool.ts`) and sat
  there forever — no promote ever arrived, because nothing was supposed to
  promote a startup pool window; those exist purely as a pre-warmed reserve
  for future tab tear-off (`agentmux-cef/src/commands/window_pool.rs`'s
  module doc).
- Reproduced independently on an **unrelated `dev:main` instance that had
  been running on the same machine for 4+ hours** with the identical stuck
  state — ruling out anything specific to one particular launch/session.

## Root cause

`agentmux-cef/src/client/navigation.rs`'s `on_load_end` handler is
responsible for calling `window.show()` on the top-level (non-pool) window
once its page finishes loading:

```rust
if !is_pool_window {
    if let Some(bv) = browser_view_get_for_browser(browser_cloned.as_mut()) {
        if let Some(window) = bv.window() {
            if window.is_visible() == 0 {
                window.show();
                ...
            }
        }
    }
}
```

Both `browser_view_get_for_browser(...)` and `bv.window()` can return `None`
— confirmed from the log: the main window's page genuinely loaded fine
(`Injected IPC port ... window_transparent=0` — main's URL never carries a
`windowLabel` param, unlike pool/floating windows, so that's main's
signature) and the frontend painted (`[startup-bench] frontend-painted`), but
the native HWND was never shown. When either `Option` resolves to `None` at
this exact moment, the old code **silently did nothing** — no log line, no
retry, no fallback. The window then stays hidden forever with zero
diagnostic trace, and whatever OTHER window happens to be on-screen (a pool
window, if its off-screen parking at `(-32000,-32000)` doesn't hold on that
particular session/monitor layout) is what the user actually sees.

This is the same class of CEF Views timing quirk already diagnosed and
worked around **on the pool-window path specifically**:
`window_pool.rs`'s `POOL_HWND_CACHE` was built after
`SPEC_POOL_WINDOW_HWND_NULL_2026_05_06.md` found that
`BrowserHost::window_handle()` returns null after the page loads even though
the underlying Win32 window is alive — the pool path caches the HWND at an
earlier, reliable point and falls back to it. The main/top-level window path
never got the equivalent treatment.

## Why this wasn't caught earlier

- It's intermittent — most `task dev` launches presumably work fine, so this
  only surfaces under specific timing (this dev machine runs many concurrent
  agent processes and CEF instances, which is exactly the kind of load that
  makes rare timing windows land).
- The failure is **completely silent**. Nothing in the logs at any level
  (`ERROR`, `WARN`, or otherwise) indicated the show() call was skipped —
  the only symptom was the visible window being the wrong one (a stray pool
  window) with no link back to the real cause.
- This directly explains a standing gap noted on PR #2525
  (`fix(menu): submenu positioning flash...`): the author flagged that
  manual `task dev` verification "didn't yield a distinguishable
  build/process to verify against" on this same shared machine and asked for
  a follow-up manual pass that never happened. This bug is a strong
  candidate for *why* — anyone hitting it would see an apparently-broken
  blank window with no error to investigate, and reasonably give up on
  manual verification rather than debug the launcher itself.

## Fix

Converted the silent no-op into a logged, bounded retry
(`navigation.rs`): if `browser_view_get_for_browser`/`bv.window()` aren't
resolvable at `on_load_end`, log a `WARN` and reschedule the same check on
the CEF UI thread every 50ms, up to 10 times (~500ms total). If it still
hasn't resolved after all retries, log an `ERROR` with enough context to
actually diagnose a next occurrence (label, retry count) — previously there
was nothing to look at all. `ShowWindowRetryTask` mirrors the existing
`PaintGateRevealTask`/`FirstPaintSignalTask` retry pattern already used in
this same file for Linux's paint-gating.

Not fully root-caused: *why* `browser_view_get_for_browser`/`bv.window()`
return `None` at this point isn't pinned down (same as the pool-window
analogue, which also ships a workaround rather than a root fix). The retry
is a pragmatic mitigation matching the precedent already set by the
pool-window code, not a guarantee it can never recur — if `ERROR`-level logs
from `ShowWindowRetryTask` start showing up in the wild, that's the signal
to dig further into *why* CEF Views loses the reference, rather than just
raising the retry budget.

## Verification

Manual `task dev` verification is exactly what this bug was blocking, so
verifying the fix itself needs a live run — tracked as a follow-up to this
retro rather than closed out here.
