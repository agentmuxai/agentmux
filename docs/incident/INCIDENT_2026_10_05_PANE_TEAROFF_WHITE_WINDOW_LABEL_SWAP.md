# INCIDENT 2026-10-05 — pane tear-off produced a blank white floating window: two pool browsers created together got each other's labels

**Status:** implemented — root cause identified (§4); fix 1 of §6 (bind each browser to its own creation) and the failure-path cleanups ship with this document. Fixes 3 and 4 remain proposals; fix 2 is no longer needed (§6).
**Date:** 2026-10-05, 13:16:50 host local (UTC−7) / 20:16:50 UTC.
**Author:** AgentY (Claude Opus 5.5), from the installed instance's logs and read-only window queries (nothing was attached to or driven).
**Severity:** Medium-high. The tear-off left the operator with an empty window instead of their pane. Nothing was logged as an error and nothing recovered. The same mislabelling is frequent (§5), so any pool-backed tear-off or new window can land on it.
**Affected:** installed v0.59.9 (latest release), Windows. Mislabelling seen in the retained logs of every version from v0.58.2 (2026-09-29) on.
**Fixed in:** the change this document ships with (first release after v0.59.9). Until then no release had touched the label binding: no commit between `v0.59.9` and this fix changes `crates/cef/src/client/`, `crates/cef/src/reducer/`, the window or pane pool, the floating-pane window or the frontend's pool bootstrap.
**Related:** `SPEC_FLOATING_PANE_POOL_RELABEL_2026_06_30.md` (pool relabel on promotion), `SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md` (snapshot/reveal handshake).

---

## 1. Summary

AgentMux keeps two pools of hidden, pre-loaded windows: a **window pool** (whole windows, for tab tear-off and new windows) and a **pane pool** (frameless floating panes, for pane tear-off). When a browser finishes being created, the host's `on_after_created` takes the **oldest pending creation** off a FIFO queue (`DequeuePendingWindowCreation`) and registers the browser under that entry's label.

At 18:56:25 UTC both pools were refilled in the same millisecond, pane pool first. Chromium finished creating the window-pool browser first. So the window-pool browser was registered as `floating-pool-2aab9711…`, and the pane-pool browser, the one inside the floating frame, as `window-pool-383ee59…`.

At 20:16:50 the operator tore a pane off. The host:
- took the pane pool's cached **frame** HWND (correct, cached separately at spawn);
- moved and showed that frame;
- sent `pool:pane-promote` to the browser registered as `floating-pool-2aab9711…`, i.e. the **window-pool page**, which only listens for `pool:promote` and ignored it.

The real pane page inside the frame never heard anything, and its browser child window stayed hidden. The frame showed white. Both host-side timeouts (80 ms snapshot ack, 2 s reveal) expired without consequence.

## 2. Timeline (UTC; host local = UTC − 7 h)

| UTC | Event | Source |
|---|---|---|
| 18:52:25 | Memory pressure: the pane-pool window and both window-pool windows evicted | host log `pool:pane`, `dnd:tearoff:pool` |
| 18:56:25.6158 | Refill: `spawning pane pool window floating-pool-2aab9711…`, then 0.08 ms later `spawning pool window window-pool-383ee59…` | host log |
| 18:56:25.6193 | Pane pool's own frame created: top-level HWND `0x1240b00` (class `AgentMuxFloatingPane-…`) | host log `[focus-restore]` |
| 18:56:25.6278 | **First** browser created → dequeued label `floating-pool-2aab9711…`; its top-level is `0x1940e6c` (`Chrome_WidgetWin_1`, a CEF Views window, i.e. the window-pool window) | host log `[on-after-created]`, `[window-edge-resize] … HWND 0x1940e6c label=floating-pool-2aab9711…` |
| 18:56:25.6448 | **Second** browser created → dequeued label `window-pool-383ee59…`; its HWND is `0x15101c` (`CefBrowserWindow`, child of the pane frame `0x1240b00`); gets `[pane-zorder]` / pane subclassing (pane client) **and** a `[close-routing]` hook (installed from the label) | host log |
| 18:56:25.85 | The pages load with their own correct URLs: `windowLabel=window-pool-383ee59…&pool` and `windowLabel=floating-pool-2aab9711…&pane-pool=1`; host warns `client claims browser-pane but this browser is a top-level window (cloned client)` | host log |
| 18:56 → 20:16 | All three pool pages tick `[atom-cache-diag]` every 30 s (3 empty-cache ticks per 30 s) | host log |
| 20:16:50.488 | `promoting pane pool window (Windows)` `floating-pool-2aab9711…` → relabel → `pool:pane-promote` emitted to the browser under that label | host log |
| 20:16:50.572 | `showing floater after its snapshot`, `acked: false`; frame `0x1240b00` shown | host log, Windows events |
| 20:16:52.574 | `refill: no reveal reported, refilling anyway` | host log |
| after | No `pool:pane-promote received` from any page; empty-cache ticks go from 3 to 4 per 30 s (the old three plus the new pool page): every old page is still alive, none mounted the pane | host log |
| 21:5x (read-only check) | Frame `0x1240b00` visible at the tear-off position, title `AgentMux — floating-pool-2aab9711…`; its browser child `0x15101c` **visible=False**. `0x1940e6c` hidden, parked at (−26214, −26214), 1200×800 | `GetWindowRect` / `IsWindowVisible` |

## 3. How labels are bound today

- Every window creation first enqueues a `PendingWindowCreation { label, kind }` (`EnqueuePendingWindowCreation`).
- `on_after_created` (`crates/cef/src/client/lifecycle.rs`) dispatches `DequeuePendingWindowCreation`, which pops the **front** of that queue, and registers the new browser under the popped label.
- That is only correct if browsers finish creation in the order their creations were enqueued. Two creation paths race here:
  - the pane pool creates its own Win32 frame and a windowed CEF browser inside it;
  - the window pool goes through CEF Views (`browser_view_create` + `window_create_top_level`).

  Neither path orders `OnAfterCreated` relative to the other, and in this log the later-requested Views browser came back first.
- Everything else keys off the label: the reducer's `browsers` map, `get_browser(label)` for every event emit, window kind (top-level vs floater), the WM_CLOSE routing hook, the promote path. The pane pool's **frame** HWND is the one thing cached by its own spawn task, not by label, which is why the right frame was shown while the event went to the wrong page.

## 4. Root cause

**Label-to-browser binding by FIFO order in `on_after_created`.** When a pane-pool and a window-pool window are created concurrently, their browsers can complete in either order and each gets the other's label. A later promotion of either label then drives the wrong browser:
- **Pane tear-off:** the frame is shown, the promote event goes to a window-pool page that ignores it; the result is a blank white window (this incident).
- **Window-pool promotion** (tab tear-off, "Open in New Window"): the promote goes to the pane-pool browser.

Concurrent refills are routine: memory-pressure eviction empties both pools at once, and the refill spawns both in the same tick. That is why the failing window was old (it was spawned by such a refill), not because age itself matters.

Not the cause, checked and ruled out:
- *The edge-tear path* (the pane spanned the column, so the source window shrank): the mother resize runs after the promote and touches only the source window.
- *A frozen or dead renderer:* every pool page kept running timers through and after the tear-off.
- *The 2026-09-22 renderer deadlock:* its fix (tracer-free CEF runtime) is in v0.59.9, and the page wasn't stuck.

## 5. How often

A browser registered under a `window-pool-*` label that then gets pane subclassing (`[pane-zorder]`), or under a `floating-pool-*` label that doesn't, is mislabelled. Counting that across every retained host log on this machine:

| Version (date) | Pool registrations | Mislabelled |
|---|---|---|
| v0.55.42 – v0.58.1 (09-12 → 09-29) | 9 | 0 |
| v0.58.2 (09-29) | 6 | 2 |
| v0.59.1 (10-01) | 7 | 4 |
| v0.59.2 (10-01, 10-02) | 8 | 5 |
| v0.59.4 (10-02 → 10-04) | 300 | 198 |
| v0.59.7 (10-03, 10-04) | 189 | 169 |
| v0.59.8 (10-04, 10-05) | 42 | 30 |
| v0.59.9 (10-05) | 38 | 23 |

Startup-only logs (three registrations, one per pool window spawned at launch) show no swaps; the swaps come in pairs at refills. A tear-off only fails if it promotes a mislabelled window, so most of these were never noticed. When the swaps began (v0.58.2 here) is worth bisecting: it probably lines up with when both pools started refilling together, such as memory-pressure eviction, rather than with the FIFO itself.

## 6. Fixes

1. **Bind the label to the browser, not to arrival order. Implemented** (`crates/cef/src/client/creation_labels.rs`). `on_after_created` now takes the pending entry for this browser's own label (`HostCommand::TakePendingWindowCreation`), wherever it is in the queue. It learns the label one of two ways:
   - a browser with a handler of its own carries the label in it (`AgentMuxHandler::new_for_creation`): floating panes, the Windows pane pool, browser panes;
   - a CEF Views window (window pool, new windows, tab tear-off, and the macOS/Linux pools) shares the top-level client, so its `BrowserView` is tagged with a view ID right after `browser_view_create`. The browser is only created inside `window_create_top_level`, so the tag is in place when `on_after_created` reads it back. (`BrowserViewDelegate::OnBrowserCreated` would have been the obvious hook, but CEF calls it *after* `OnAfterCreated`.)

   Popping the queue head remains only as a logged fallback (`no pending creation names this browser`). The three failure paths that cleaned up with a head pop (browser pane, floating pane, pane pool) now remove their own entry by label.
2. **Assert the kind at registration.** Not needed with (1): every creation path now names its browser, so a pool label can no longer land on the other pool's browser.
3. **Make the promote fail safe.** If the promoted page hasn't acknowledged within a short deadline (e.g. 1.5 s), destroy the window and open the pane through the cold path (`?floatingPaneId=…&workspaceId=…`). This would have turned this incident into a slower tear-off, and it covers causes other than this one.
4. **Make delivery observable.** Log the target browser's identity (browser id and HWND) on every promote emit, and tag frontend log lines with the window label; both would have made this RCA a five-minute read.

**Until fixed:** a running instance that has mislabelled pool windows keeps them until they are promoted or evicted. Restarting AgentMux clears both pools.

## 7. Evidence

- Host log: `~/.agentmux/channels/<instance>/versions/0.59.9/logs/agentmux-host-v0.59.9.log.2026-10-05` (targets `pool:pane`, `dnd:tearoff:pool`, `agentmux_cef::client::lifecycle`, `agentmux_cef::client::wndproc`, `host-reducer`).
- CEF log: `…/logs/cef-debug.log`: no detached-frame or crash warnings at the tear-off.
- Window state, read with `IsWindowVisible` / `GetWindowRect` / `EnumChildWindows` at about 21:50 UTC, while the white window was still open.
- The counts in §5 come from a script that scans every retained host log for pool registrations and the pane-subclass marker that follows each.
