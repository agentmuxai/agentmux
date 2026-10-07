# Browser panes during a window resize: why they ghost, and what to rebuild

**Date:** 2026-10-06
**Status:** analysis. Measurements in §2, how it works today in §3, causes in §4, options in §5, recommendation and plan in §6. §5.1 is implemented in the PR that adds this doc; results in §8.
**Author:** Agent4
**Trigger:** Repo owner, 2026-10-06: *"when resizing a window tab full of browser panes, there is heavy latency. The DOM on each pane appears as a slow moving ghost lagging behind the resize. So the browser pane is actually quite touchy, and it may need a rethink. There are a lot of edge case patches like panels that expand over the DOM. Take a serious look at the current structure. Write a report that puts performance above all else."*
**Related:** `ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md` (the DOM side of resize, now at or near 60 fps), `SPEC_NATIVE_BROWSER_PANE_2026_04_17.md` and `reports/BROWSER_PANE_DEFINITIVE_2026_04_17.md` (why panes are native), `ANALYSIS_BROWSER_PANE_AIRSPACE_ARCHITECTURE_2026_05_30.md`, `SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md`, `architecture/PANE_LAYOUT_AND_REFLOW_ARCHITECTURE.md`.

Code citations are against `main` @ `669337940`. Measurements are Windows only; macOS and Linux use a different host path (§3.1) and weren't measured.

---

## 1. Summary

- **The ghost is a queue, not slow drawing.** Each browser pane reports its rect to the host with its own HTTP request, every frame it changes. Nine panes in a fast drag sent **324 requests in 1.7 s**, with **up to 111 in flight at once**. The host applies them one at a time, at about 8 ms each, by a cross-thread `SetWindowPos` that waits for the UI thread. So the queue grows for as long as the drag lasts: **median 542 ms, p90 738 ms** from request to reply. A native pane is drawn where a request from about half a second ago put it.
- **A request on its own is fast: 7–9 ms.** The cost is volume, not round trip. Nine panes at 60 fps is 540 requests a second, and the host gets through about 190.
- **Capping each pane at one request in flight, newest wins** (a frontend-only experiment, not committed) took the median from 542 ms to 96 ms. The panes are still behind, though, and each one is at a different stale position, because they are applied one by one.
- **The fix that matters:** one batched update per window per frame carrying every pane's rect, with at most one in flight. The host applies the batch in one UI-thread task with `DeferWindowPos`, so all panes move together. The expected trail is one or two frames, coherent across panes. It keeps today's architecture and is days of work, not weeks.
- **The second cost is the GPU.** Each pane is its own top-level compositor output: its own renderer process and its own swap chain, presented separately. In the drag, the GPU process's main thread was busy **72%** of the time (1,510 of 2,086 ms), with about 300 `Present` calls in two seconds and 300 ms of raster. That is a cost per pane, independent of the IPC fix.
- **The strategic option** is to host panes as CEF Views `BrowserView`s on Windows too, which is what Linux and macOS already do. They would be composited inside the main window's own compositor instead of being separate child HWNDs. That should remove the per-pane swap chain, the wrapper HWND and the `SetWindowRgn` clipping, but it needs a spike to confirm on Windows (§5.2).
- **iframes would be the fastest and remove every airspace workaround**, but they break real browsing: framing headers, partitioned cookies and logins, and a joint back/forward history across all panes in a window. Use them only for content the app controls (§5.3).

## 2. Measurements

All on Agent4's isolated `task dev` window, Windows, 1986×1567 physical px. The tab holds nine browser panes on `https://agentmux.ai/`, plus the agent list, a CPU chart and Swarm. The "fast drag" grows the window 30 px every 16 ms for 30 steps, through `SetWindowPos`, not Windows' modal size loop.

### 2.1 What it looks like

Captures with `PrintWindow(…, PW_RENDERFULLCONTENT)` show the window's own content, even with other windows on top.
- **Right after the last step of the fast drag:** the DOM, meaning every pane header, URL bar and border, is laid out at the new width. The native browsers are still where they were several steps earlier, about 200 px to the left. They cover the CPU and Swarm panes and leave the right column's pane bodies empty, showing the placeholder `#222`.
- **600 ms later:** everything lines up.

### 2.2 How long a native pane takes to arrive

- **One 300 px grow step,** then polling a pixel inside the top-right pane until it shows page content instead of the placeholder: **26–93 ms**, five runs. Each capture takes about 35 ms, so that is coarse.
- **In a continuous drag** the trail grows instead of staying constant (§2.1). That points to a queue.

### 2.3 The rect updates (page side, `fetch` wrapped over CDP during the fast drag)

| | Requests | Requests in flight, max | Request → reply p50 / p90 / max |
|---|---|---|---|
| Today | 324 in 1.7 s (~187/s) | 111 | 542 / 738 / 771 ms |
| Experiment: one in flight per pane, newest wins | 166 in 1.2 s | 18 | 96 / 136 / 153 ms |
| One request with nothing queued | | 1 | 7–9 ms |

### 2.4 The GPU process during the same drag (Chromium trace, all processes)

| Thread | Busy |
|---|---|
| GPU process main thread | 1,510 of 2,086 ms (72%), 14,228 tasks |
| …of which `SkiaOutputSurfaceImplOnGpu::SwapBuffers` / `DXGISwapChainImageBacking::Present` | ~295 / ~256 ms, about 300 presents |
| …of which raster (`RendererRasterWorker`, `DoEndRasterCHROMIUM`) | ~300 ms |
| Each browser pane's renderer main thread | ~260 ms |
| The app's own renderer main thread | well under one frame per frame for the DOM (see the related analysis) |

## 3. How it works today

### 3.1 The host (`crates/cef/src`)

- **Windows: each pane is a windowed child HWND.**
  - The host creates its own `WS_CHILD` wrapper HWND at the pane rect, raised `HWND_TOP` (`browser_pane/wrapper.rs:186-242`).
  - It then creates the CEF browser as a child of that wrapper with `WindowInfo::set_as_child` and the Alloy style (`browser_pane/creation.rs:175-195`).
- **Linux and macOS: a CEF Views `BrowserView` added with `window.add_overlay_view`** (`browser_pane/creation_views.rs:199-205`). On macOS that overlay is its own `NSWindow`, sized through raw ObjC (`:233-366`).
- **Every pane is a separate CefBrowser with its own renderer process** (`app/mod.rs:1051-1060`). No frame-rate cap or background throttling is set.
- **Applying a rect (Windows).**
  - `browser_pane_resize` is an async axum route on a tokio worker (`ipc.rs:525-540`).
  - It calls `BrowserPaneManager::resize` → `resize_wrapper` → `SetWindowPos(wrapper, …)` **directly from the tokio thread** (`browser_panes/navigation.rs:27-52`, `browser_pane/wrapper.rs:247-259`). The wrapper belongs to the UI thread, so Win32 sends the call to the UI thread and blocks until its message loop handles it.
  - The wrapper's `WM_SIZE` then resizes CEF's own HWND (`wrapper.rs:98-114`).
  - Last comes `notify_move_or_resize_started()`.
  - That is one native move per request, with no batching or coalescing.
- **Linux** posts one UI task per request (`navigation.rs:56-64`). **macOS** posts one per request, plus a 50 ms "reaffirm" that re-applies the rect captured when the message arrived (`ui_tasks/pane_geometry.rs:818-829`). During a live drag, that reaffirm snaps the pane back to a stale rect (`SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md` F2).
- **The host does nothing to panes when the main window resizes** (`client/wndproc.rs:565-743`). Panes move only when the page reports a rect.

### 3.2 The frontend (`frontend/app`)

- **`usePaneRectSync`** (`view/browser/use-pane-rect-sync.ts`) measures the placeholder with `getBoundingClientRect()` × DPR and calls `getApi().browserPanes.resize(blockId, rect)` without awaiting it (`:149`). Its only coalescing is a "same as the last rect sent" check (`:139-147`).
- **Four things trigger it:**
  - a ResizeObserver on the placeholder (`:228`);
  - a 200 ms `setInterval` safety net (`:230`);
  - a per-frame reflow sampler opened by `notifyPaneReflow()` (`:159-175`, Windows only, `TileLayout.win32.tsx:81`);
  - visibility, which sends 0×0 to hide (`:138`).
- **The transport** is `invokeCommand("browser_pane_resize", …)`, an **HTTP POST to `127.0.0.1/ipc`** with a bearer token (`platform/ipc.ts:32-120`, `host/cef-host-commands.ts:15-16`). There is one request per pane and no cap on how many are in flight. Chromium allows six HTTP/1.1 connections per host, so the rest queue in the network stack before they even reach the host.

### 3.3 Airspace: everything that exists because a pane is a native window above the DOM

| Mechanism | Where | Size |
|---|---|---|
| Overlay-clip registry: DOM overlays cut holes in panes (`SetWindowRgn` on Windows, a `CAShapeLayer` mask on macOS, hiding the whole pane on Linux) | `platform/pane-overlay.ts`; host `browser_panes/clip.rs:57-657` | one batched IPC per window, waits up to 300 ms for a freeze frame first |
| `usePaneOverlay(el)` opt-in | modal, flyout menus, popovers, the more-dropdown, the lightbox, agent decision and question panels, the browser view itself | 11 call sites |
| `data-pane-overlay` auto-discovery: body MutationObserver plus per-element observers | `platform/pane-overlay-auto.ts` (Windows only) | ~15 tagged elements in 13 files |
| Imperative `registerPaneOverlay` | the JS context menu, `util/cef-api.ts` | 2 call sites |
| Menus placed away from panes (`avoidNativePanes`), plus a dev "behind native pane" warning | `util/menu-position.ts` | every menu |
| Freeze-frame JPEG shown while a pane is hidden (Linux) | `view/browser/use-freeze-frame.ts` | 4 s auto-drop, 250 ms deferred clear |
| Drag snapshot plus a full-pane hole during tile, tab or pane drags | `view/browser/browser-view.tsx`, `use-drag-snapshot.ts` | 500 ms cap, 3 s prewarm age |
| The wrapper HWND must be clipped too, or it swallows clicks inside a hole | `browser_panes/clip.rs:123-149` | `ANALYSIS_WINDOWS_PANE_OVERLAY_WRAPPER_HITTEST_GAP_2026_07_13.md` |
| Focus: HWND subclassing redirects `WM_SETFOCUS`, rate-limited, reinstalled after every navigation | `browser_pane/hwnd.rs:291-590`, `callbacks.rs:495-510` | Windows |
| macOS: key-window swizzles, `acceptsFirstMouse`, `ignoresMouseEvents` toggling | `ui_tasks/platform_macos.rs`, `pane_geometry.rs` | macOS |

None of this is about resize performance directly, but all of it is the price of "a native window above the DOM". It is also why every new floating UI element needs to remember to opt in.

## 4. Causes, ranked by impact on the resize

1. **Unbounded, per-pane, serialized rect updates (§2.3).** One HTTP request per pane per frame, nothing dropping superseded rects, and a host that applies each one with a blocking cross-thread `SetWindowPos`. Throughput is about 190 a second against demand of 540, so lag grows with drag length and lands at roughly half a second. **This is the ghost.**
2. **Panes move independently.** Even with the queue gone (the experiment), each pane lands at a different moment, so a drag shows the panes out of step with each other as well as with the DOM.
3. **Every pane is a separate compositor output (§2.4).** A window resize means every pane reallocates its surface, re-rasterizes and presents its own swap chain, all through one GPU main thread. With nine panes that thread was 72% busy, so even perfectly timed rects would land on panes that draw late.
4. **The page and the panes are composited separately, by DWM.** A DOM frame and a native move can't be made atomic. With the queue gone, the floor is a trail of one or two frames, not zero.
5. **macOS's 50 ms reaffirm** re-applies stale rects during a drag. It isn't measured here, but by construction it fights a live resize.

## 5. Options

### 5.1 Keep native child windows; fix the update pipeline (recommended first)

- **Frontend: one update per window per frame.**
  - A per-window scheduler collects every pane's rect in a single `requestAnimationFrame` and sends one `browser_panes_set_rects {window_label, rects:[{block_id, x, y, w, h}]}`.
  - At most one request is in flight. While it is in flight, newer rects replace what's pending, and when it returns the newest batch goes.
  - `usePaneRectSync` keeps measuring but stops sending. Tab visibility (0×0) and the 200 ms safety net go through the same scheduler.
- **Host: apply the batch in one UI-thread task.**
  - The IPC handler posts the batch to the UI thread and returns at once, instead of calling `SetWindowPos` from tokio.
  - The UI task wraps every pane's wrapper move in `BeginDeferWindowPos` / `DeferWindowPos` / `EndDeferWindowPos` (with `SWP_NOACTIVATE | SWP_NOZORDER`), so Windows moves all panes in one operation.
  - It then calls `notify_move_or_resize_started()` once per pane.
- **macOS and Linux:** the same batch command, one UI task per batch. Drop macOS's 50 ms reaffirm while the window is resizing, or reaffirm only the newest rect.
- **Transport:** at one request per frame, HTTP's 7–9 ms round trip is acceptable. Moving this hot path to a CEF process message or the existing WebSocket would cut that to about a millisecond. That is worth doing later, not first.
- **Expected:** native panes one or two frames behind the DOM, all together, however long the drag. It is measurable with the probes in §7. The GPU cost (cause 3) is unchanged.
- **Cost:** small. One new IPC command on each side, one frontend scheduler, and the old per-pane `browser_pane_resize` kept for create and one-off moves.

### 5.2 Host panes as CEF Views `BrowserView`s on Windows too (the strategic direction; spike first)

- **What:** the same `add_overlay_view` path Linux and macOS already use (`creation_views.rs`), on Windows. The main window is already a CEF Views window, and on Windows a Views overlay widget is an Aura child window inside the same top-level compositor, not a separate HWND. That needs confirming in the spike.
- **What it should buy:**
  - **One compositor and one swap chain for the window.** Viz aggregates each pane's surface into the window's frame, the way Chrome draws its side panels and docked DevTools. That removes cause 3's per-pane present and makes the bounds and the content land in the same frame.
  - **No wrapper HWND and no HWND subclassing.**
  - **Bounds set on the UI thread**, through `CefView::SetBounds` from the same batched task as §5.1.
- **What it doesn't fix by itself: airspace.** An overlay view still draws above the app's own `BrowserView`, so DOM menus still can't cover a pane. Clipping would move from `SetWindowRgn` to a view-level mask. CEF's public `CefView` API has no clip path, so that likely needs a small patch in the `agentmuxai/cef` fork (a clip rect or mask on the overlay's layer). The frontend overlay registry (§3.3) would stay, pointed at the new clip.
- **Risks:**
  - CEF issue #3790 (an overlay `BrowserView`'s renderer never initializing) was the April blocker. Linux and macOS ship on this path now, but Windows needs to be checked.
  - Focus and IME behave differently in a Views overlay.
  - Opaque backgrounds only (cef#4035).
- **Spike to decide (2–3 days):**
  1. One pane on Windows through `add_overlay_view`.
  2. Measure presents per frame with 9 panes (§2.4), the trail during the fast drag (§2.2), and GPU main-thread load.
  3. Try a layer clip in the fork.

### 5.3 iframes (fastest by far, wrong for general browsing)

- **Performance:** cross-origin iframes run as out-of-process frames, but their surfaces are embedded in the app's own frame by Viz. Resizing is the DOM's own layout, with perfect sync, one swap chain and no IPC. **Every airspace mechanism in §3.3 would be deleted**, because a menu just draws on top.
- **Why April ruled them out, and what still holds:**
  - **Framing headers.** `X-Frame-Options` and CSP `frame-ancestors` refuse framing on most large sites. The April crash, where the error was taken as a top-level navigation failure, was a handling bug. The refusal itself is real. Removing those headers needs interception at the network layer, which must be verified in CEF before it's assumed possible.
  - **Third-party cookies and storage are partitioned in a frame**, and Google and others refuse to sign in inside frames. Logged-in browsing would break.
  - **One back/forward history per window.** Frames share the tab's joint session history, so Back in one pane could navigate another.
  - **No per-pane process control or survival.** Reloading the app reloads every pane, and moving a pane to another window reloads it. Today's `keepAlive` native panes survive both.
- **Verdict:** keep native panes for the web. Consider an iframe pane only for content the app controls and frames safely, such as localhost previews or the app's own docs, where it would be the fastest pane type there is.

### 5.4 Off-screen rendering (OSR) into the page: not recommended

Rendering panes windowless and painting their frames into the app means copying every frame through the host (or plumbing shared textures into the page, which Chromium doesn't support without engine patches), plus forwarding all input by hand. It costs more per frame than §5.1 or §5.2, for no gain over §5.2.

### 5.5 A "resize mode" that freezes panes during a drag (fallback only)

On `windowresize:begin`, show each pane's last screenshot, scaled, in its placeholder and hide the native pane, then restore on `windowresize:end`. The DOM then moves alone, in perfect sync, and the GPU does almost nothing for panes during the drag. But content is static while you drag, and it needs a screenshot ready at drag start (the `use-drag-snapshot.ts` prewarm exists). Keep this in reserve if §5.1's trail is still visible.

## 6. Recommendation and plan

Performance first, in order:

1. **§5.1, the batched pipeline.** Days of work, keeps the architecture, removes the half-second ghost.
   - **Target:** during the §2 fast drag, request → reply p90 under 20 ms with one request in flight, and every pane within two frames of the DOM in every capture.
   - **Also:** fold macOS's reaffirm into the batch.
2. **The §5.2 spike, Windows on Views overlays.** Decide on numbers:
   - presents per frame (target: one per window);
   - GPU main-thread load with 9 panes (target: well under 50%);
   - the trail.

   If it holds up, migrate Windows to it. That lets the wrapper HWND, HWND subclassing and `SetWindowRgn` clipping go, with clipping moved to a fork patch shared by all three platforms.
3. **Separately, cap the cost of panes you can't see.** Hidden-tab panes (0×0) still have live renderers. Throttle or suspend their rendering (`WasHidden`) so a window with many tabs of panes doesn't pay for them, and so memory pressure stays down (the test machine was warning that its page file was nearly full).
4. **Don't migrate web browsing to iframes.** Keep §5.3 in mind for app-controlled content only.

## 7. How this was measured

The scripts are in Agent4's workspace, not in the repo:
- **`dragshots.ps1`:** grows the window in steps and saves `PrintWindow` captures mid-drag and after settling.
- **`nativelag.ps1`:** one grow step, then polls a pixel inside a pane until the native browser arrives.
- **`traceprobe.mjs`:** a Chromium trace of all processes during the drag.
- **IPC timing:** an inline CDP snippet that wraps `window.fetch` and times each `browser_pane_resize` request.
- **The experiment in §2.3** was a local, uncommitted change to `use-pane-rect-sync.ts`, reverted afterwards.

## 8. Results: §5.1, and what else was filling the connections

Implemented in the PR that adds this doc:

- **Frontend:** `view/browser/pane-rect-batcher.ts`.
  - A pane records its newest rect.
  - Once the current task ends (a `MessageChannel` task, so every pane's ResizeObserver callback for the frame has run), the window sends everything in one `browser_panes_set_rects`, with at most one in flight. When it returns, whatever is newest goes at once.
  - All of a pane's rects go through it, including the 0×0 that hides an inactive pane, so a stale rect can never re-show a hidden pane.
  - On a host without the command it falls back to per-pane `browser_pane_resize`.
- **Host:** `browser_panes/bounds.rs`.
  - A batch merges into the window's pending rects (newest per pane), and one UI task per window drains them.
  - On Windows, every wrapper HWND moves in one `BeginDeferWindowPos`/`EndDeferWindowPos` per parent, with the flags `resize_wrapper` uses (falling back to per-pane `SetWindowPos` if a deferred move fails). Linux and macOS run their usual Views resize for each pane inside that one task.
  - A pane is moved only while it is live and still in the window that sent the batch. The IPC call answers once the panes have moved, or after 250 ms.

**With batching alone, each batch still took ~90 ms.** Timing inside the host showed the UI task starting within a millisecond of being posted and moving all nine panes in 13–26 ms (about 2 ms a pane; `notify_move_or_resize_started` 0.2 ms). The rest was before the host. Timing every `/ipc` request during the drag found **837 `fe_log_structured` requests in 1.5 s, with up to 95 IPC requests in flight**. Every `console.*` line went to the host as its own request, so the pane batch waited for one of Chromium's six connections. The source:

- CEF reports a pane's favicon URLs again on every viewport change of this page, so on every frame of a drag, and the host forwarded each report.
- All nine panes' models logged every pane's favicon and title events, the eight `match=false` ones included.
- That made 744 log lines for about one second of dragging, against 1 line in 3 s idle.

Also fixed in this PR:

- **`frontend/log/log-pipe.ts`** batches console lines into one `fe_log_batch` request every 100 ms, at most one in flight. It holds at most 2,000 lines and reports how many it dropped. A host without the command gets per-line requests as before.
- **`client/display.rs`** sends a pane's favicon list only when it, or the page URL it belongs to, changed.
- **`browser-model.ts`** has a pane log only the favicon and title events addressed to it.

**Measured**, the §2 fast drag on a tab of nine browser panes, three runs:

| | Rect requests | Request → reply p50 / p90 / max | IPC requests in flight, max | Log requests |
|---|---|---|---|---|
| Before | 324 in 1.7 s | 542 / 738 / 771 ms | 111 | ~750 a second of dragging |
| After | 22 per drag (each carrying all 9 panes) | 21–22 / 28–34 / 33–37 ms | 3 | 8–9 batches per drag |

A `PrintWindow` capture taken right after the last step of the fast drag now shows every native pane on its own placeholder, all in step with each other and the DOM. Before, the panes sat about 200 px behind, over the neighbouring DOM panes.

**What's left** is the host's move itself: about 2 ms per pane, so ~20 ms for nine, which caps updates near 45 a second. That cost lives in Chromium's handling of each child window's resize. Moving panes into the window's own compositor (§5.2) is the way past it, and the per-pane GPU cost of §2.4 is unchanged by this PR.
