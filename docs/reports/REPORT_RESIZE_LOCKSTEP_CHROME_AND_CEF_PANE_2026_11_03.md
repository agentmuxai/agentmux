# Report: Can the chrome and a CEF browser pane resize in lockstep, with the chrome waiting for the browser's frame?

**Date:** 2026-11-03. Code and measurements against AgentMux `origin/main` at `ce93302` (0.60.13 dev build), Windows 11, one 3840x2160 display at 59 Hz, 125% scale.
**Task:** #21066 (Research). **Author:** Agent2-5@narko.
**Status:** active. Option (b) is built as an opt-in on Windows (`AGENTMUX_PANE_BACKEND=views`) in the same pull request that ships this report, and became the Windows default with task #29711 (`AGENTMUX_PANE_BACKEND=hwnd` goes back to child windows); the measurements here are from before it, and the pending re-run (section 8) was replaced by the operator testing the build themselves.
**Readers:** AgentY (front door), Korp (browser engine, asked to read this draft), whoever builds from it.
**Publishing note:** ships in the pull request that builds option (b). It cites no private-repo material and names no internal host.

The operator's words (2026-11-03): "curious, when resizing, is it possible to lock the agentmux chrome/pane border to the browser pane DOM? Essentially, the agentmux layer would halt for the browser CEF/renderer dom before moving. It would be resize would be slower on the agentmux side, when browser panes are active, because it would need to match the rate of the cef growth. but that is better than the panes being quicker to resize, but the cef lagging. research best practice regarding this"

---

## 0. The decision, and the recommendation

**The decision this feeds:** should AgentMux synchronise a resize so that the pane border/chrome and the CEF browser view move in the same frame (holding the chrome until the browser has a frame at the new size), and if so how, on Windows first, at what cost to smoothness and complexity.

**Recommendation: do not build the chrome-waits-for-browser gate (option a) as the fix. Measure the case the operator sees first, then spike the one approach that can really be atomic: hosting panes in the main window's own compositor (option b), with a frame-wait deadline.**

Why, in the order of weight:

1. **On current `main` the lag the operator describes is not the one we measured; it is the other way round.** In a stepped drag on a 1, 3 and 6 pane tab, the native pane box moves **ahead of the chrome** by about **1 display frame** (1 pane), **1.3** (3 panes) and **1.5** (6 panes) at median, with the worst frames at 2 to 4 frames (section 2). The host already moves the panes itself from the window's `WM_SIZE`, stretching the page's last rects, before the page has painted. The browser's content then stays within about half a frame of the pane box on a light page. So the chrome (the main window's page) is the slowest layer. A gate that makes the chrome wait for the browser makes the slowest layer slower.
2. **The chrome cannot wait in a way that looks right.** The chrome is a Chromium page painted by the main window's own compositor, and the OS window frame under the user's pointer moves regardless. "Holding" the chrome can only mean holding the page's layout at the old size while the window frame and the browser grow around it. That is a visible gap, not lockstep. Windows only synchronises a window frame with its content if the app opts in through private DWM calls (section 3).
3. **There is no signal to wait on for a windowed pane.** Korp confirmed it: panes are child HWNDs (not off-screen rendered); the `set_rects` reply means "HWND moved", not "painted at the new size"; nothing consumes a paint callback for windowed panes. A gate needs a new signal (a renderer round trip through CDP, or a fork patch), and a hung page needs a timeout that returns to today's behaviour.
4. **What would make it atomic is one compositor.** Chromium keeps its own tab strip and page in lockstep by holding the parent's frame until the child's surface for the new size arrives, within a deadline. That works only inside one compositor tree. Windowed child panes are separate swap chains that DWM composes. The way to get the real behaviour is the strategic option the 10-06 analysis already named: panes as CEF Views `BrowserView` overlays on Windows (what macOS and Linux already do), with the surface-synchronisation deadline raised so the compositor waits for the pane's frame. Electron hit exactly this problem on macOS (the window frame reaches the screen with the page's previous frame) and fixed it with a patch that makes the resize wait for the page's frame: 0 out-of-step frames in every direction, at the price that the window resizes at most at the page's frame rate.

**What to do, in order:**

1. **Finish the measurement that matches the complaint** (section 8): a stretch-aware run, plus the nine-pane tab of real pages the operator described, on a slot where the window may be in front (the leader asked that this not be done on the operator's desk).
2. **Spike option b on Windows** (2 to 3 days, the same spike Agent4's 10-06 analysis proposed; its harness scripts live in Agent4's workspace, so ask that clone's owner, not Korp): one pane through `add_overlay_view`, presents per frame, lag in frames with the harness below, and a deadline switch. Decide from numbers.
3. **Cheap interim, only if the numbers show stretch or clip is what is seen:** option c, covering the artefact, and a per-pane freeze for a pane that fails a responsiveness probe (a hung page), never a global gate.

**Biggest uncertainty:** these numbers come from a synthetic stepped resize on solid-colour test pages, not a person dragging a window over nine real pages. They show the direction of the lag and its size on light pages; they do not reproduce the operator's heavy case.

---

## 1. How AgentMux resizes a browser pane today

Read: `docs/analysis/ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md` (and its section 8 results), `ANALYSIS_BROWSER_PANE_AIRSPACE_ARCHITECTURE_2026_05_30.md`, `docs/CEF_ARCHITECTURE.md`, and the code.

### 1.1 What a pane is on each platform

| Platform | Pane | Where |
|---|---|---|
| Windows | A **windowed child HWND**: a `WS_CHILD` wrapper at the pane rect, with CEF's own child window filling it (`WindowInfo::set_as_child`, Alloy style). **Not** off-screen rendered. Each pane is its own renderer process and its own swap chain, composed by DWM. | `crates/cef/src/browser_pane/wrapper.rs`, `creation.rs:223` |
| macOS, Linux | A CEF Views `BrowserView` added with `add_overlay_view` (on macOS the overlay is its own `NSWindow`). | `browser_pane/creation_views.rs` |

The `windowless_frame_rate: 60` in `creation_views.rs` is the only off-screen-style setting in the tree and applies only to the Views path. Option b below is "make Windows use that path".

### 1.2 The path of one resize, Windows

1. The OS resizes the main window. The main page's layout follows (CSS, the tile layout) and the main window's compositor paints the new chrome.
2. **Host, ahead of the page:** the window's `WM_SIZE` runs `follow_window_resize` (`crates/cef/src/browser_panes/bounds.rs`, `LayoutBase::predict`), which moves every shown pane's wrapper to where the page's *last* layout, scaled by the size change, would put it. The page's own rects for the new size then replace the guess.
3. **Page, behind:** a `ResizeObserver` per placeholder records each pane's rect; `pane-rect-batcher.ts` sends **one `browser_panes_set_rects` per window per frame**, at most a few in flight, numbered so the host drops stale ones. The host applies a batch in one UI task with `BeginDeferWindowPos`/`EndDeferWindowPos`. It does not wait for the page's reply (the comment in the batcher says why: on a busy page the reply waits behind everything else).
4. The wrapper's `WM_SIZE` resizes CEF's child window. CEF then runs its own resize: renderer layout, paint, compositor, GPU swap chain resize, present. Until the new frame arrives the swap chain shows the old one scaled or cropped.

### 1.3 Where lag can come from

| Source | Size | Status |
|---|---|---|
| IPC volume: one request per pane per frame, serialised | 542 ms median with nine panes (10-06) | **Fixed** by the batcher and `DeferWindowPos` (22 to 37 ms per batch) |
| Log traffic filling the IPC connections | 837 requests in 1.5 s (10-06) | **Fixed** (log batching, favicon dedupe) |
| Host moving the HWNDs | about 2 ms a pane | Small; and `follow_window_resize` moves them without waiting for the page |
| The chrome's own paint after the OS resize | measured here, 1 to 1.5 frames behind the native box | **Open**; this is the dominant term in section 2 |
| CEF resize to present, inside the pane | content stretched or cropped for a few frames; a hung page stays stretched or cropped | **Open**, section 2.4 |
| GPU: each pane is a swap chain; the GPU main thread was 72% busy with nine panes (10-06) | per pane | **Open**, unchanged since 10-06 |

---

## 2. Measurement: frames of lag between the chrome and the pane

### 2.1 Method (so it can be repeated)

- **Instance:** my own `task dev` from `origin/main` `ce93302` (isolated home, a dev build with `opt-level=1`, no LTO, started at below-normal priority). No occlusion or backgrounding flags were set; the window was kept un-covered while it was captured (section 6 says what that means for the flags the leader named).
- **Layout:** one browser pane, or three, or six, side by side, all loading a local test page.
- **Test pages** (local, served on 127.0.0.1): a solid magenta page; a variant with a **busy renderer** (12 ms of main-thread work per frame and forced layouts on every resize); a variant that **hangs its main thread** 1.5 s after load (verified: a CDP call to it never returns); and, in the later variant, a 60 CSS px cyan bar pinned to the page's own right edge to tell a re-laid-out page from a stretched or cropped old frame.
- **DOM reference:** a 4 px pure-green element pinned `position: fixed; right: 12px` in the main window's own page, so it is drawn by the chrome's compositor like the pane borders.
- **Capture:** `ffmpeg` desktop duplication at 60 fps of two pixel rows (the header row, where the green marker is, and a body row across the panes), so every row comes from the same displayed frame. A pane's native edge is the right-most magenta or cyan pixel; its DOM edge is where the marker puts it.
- **Drive:** `SetWindowPos` (asynchronous, so the cadence holds) steps the right edge by **10 px per 16.7 ms frame** (and, for a fast drag, 29.6 px per frame), +400 px then back. Not the OS modal sizing loop: my first attempt with synthetic mouse input behaved erratically (the window jumped before the first step), so it was dropped. Agent4's 10-06 harness also used `SetWindowPos`.
- **Metric:** for the right-most pane, the native edge's offset from where a linear fit through the settled start and end says the DOM edge is, in pixels and in display frames (pixels / px per frame). **Sign:** the numbers below are how far the native pane is *ahead* of the chrome (negative in the raw output, shown positive here) unless stated.
- **Not measured:** a person dragging a window; the modal sizing loop; real pages; nine panes; macOS and Linux.

### 2.2 Results: the native pane box against the DOM chrome (growing, right-most pane)

Only runs whose measured DOM speed equals the intended speed are shown (marker frames missing on other runs made their speed estimate unreliable; see 2.3).

| Panes | Page | px/frame | Native box ahead of chrome, median / p90 / worst (frames) | Mean ahead (px) | Page content ahead of chrome, median / p90 / worst |
|---|---|---|---|---|---|
| 1 | light | 10 | 1.0 / 1.0 / 2.0 | 9 | 1.0 / 1.0 / 1.1 |
| 3 | light | 10 | 1.3 / 1.7 / 3.7 | 13 | 1.3 / 1.7 / 3.5 |
| 6 | light | 10 | 1.5 / 1.9 / 2.8 | 13 | 1.5 / 1.9 / 2.8 |
| 3, one hung | light | 10 | 0.4 / 0.7 / 1.4 | 5 | 0.4 / 0.5 / 1.4 |
| 1 | light | 30 | 0.0 / 1.0 / 4.9 | 15 | 0.0 / 0.2 / 4.9 |
| 3 | light | 30 | 0.4 / 1.4 / 2.7 | 26 | 0.4 / 1.4 / 2.4 |
| 3 | busy 12 ms | 30 | 1.3 / 1.7 / 2.0 | 32 | 0.7 / 1.3 / 2.0 |
| 3, one hung | light | 30 | 0.7 / 1.7 / 1.7 | 27 | 0.7 / 1.6 / 1.7 |

Shrinking: the native box is ahead by about 1 frame (1.4 to 8 px at 10 px/frame; most frames at 30 px/frame had no marker, see 2.3).

### 2.3 What the numbers say

- **The native pane leads the chrome, and the lead grows with the number of panes** (1.0, 1.3, 1.5 frames at 10 px/frame). That fits the code: the host moves the HWNDs at `WM_SIZE` in about 2 ms a pane, while the chrome is painted by the main window's compositor, which has more to paint with more panes on screen.
- **Page content tracks the pane box on a light page** (the two columns are within a pixel or two). On the busy page it is about **0.5 frame behind the box** (content 0.7 against box 1.3 at 30 px/frame), but still ahead of the chrome.
- **A hung page does not show up as a lagging box.** Its pane box still moves with the others, because the host moves the HWND and the GPU stretches or crops the last frame. A box-edge metric cannot see that; the stretch-aware metric in 2.4 can.
- **The chrome itself falls behind the OS frame.** At 30 px/frame the green marker was missing from 27 to 28 frames of each fast drag, and from 40 frames in a six-pane 10 px/frame drag (frames where the main page has not yet painted anything at the new size). So the chrome is the slowest layer in these runs, and it gets slower with more panes. This is consistent with the 10-06 finding that panes share one GPU thread.

### 2.4 Content staleness: is the page re-laid-out, or an old frame stretched or cropped?

The solid-colour pages cannot tell a re-laid-out page from a stretched old frame. A 60 CSS px bar pinned to the page's own right edge can: a stretched frame makes the bar wider than nominal, a cropped frame makes it narrower. One light pane, 10 px/frame:

| Phase | Result |
|---|---|
| Growing | The page is current in the median frame; **23% of frames are stale by up to 3.4 frames** (the bar is wider than nominal: the old frame is stretched) |
| Shrinking | The page's content is **cropped by exactly one frame's worth (10 px)** in 87% of frames |

This is the only stretch-aware run I finished (one pane); the matrix over 1, 3, 6 panes, the busy page and the hung page is written and **pending** (section 8). Until then, what is shown for a hung or busy pane under this metric is predicted, not measured: a hung renderer never re-lays-out, so its pane would show the stretched or cropped *old* frame for the whole drag.

### 2.5 Harness

Files are in my workspace, not the repo, in `resize-lab/`: `page*.html`, `heavy*.html`, `hang*.html` (test pages), `serve.py`, `drag2.ps1` (stepped resize), `capture.py` (strip capture), `analyze*.py`, `cfg.sh` and `setupN.js` (set up N panes through CDP and verify), `matrix*.sh`. They are for the build task to turn into `scripts/perf/` if wanted.

---

## 3. Prior art

Evidence quality: **Doc** = vendor documentation, **Code** = source, **Issue** = tracker or forum, **Not found** = I looked and found no primary source.

| System | What it does about frame and content staying in step during a resize | Evidence |
|---|---|---|
| **Chromium surface synchronisation (viz)** | A new `LocalSurfaceId` is allocated on resize; the parent surface depends on the child's surface, and viz waits for the child up to a deadline measured in BeginFrames, then activates anyway (a stale or blank area shows). The switch `--deadline-to-synchronize-surfaces` sets the default. A larger or unlimited deadline gives sync at the price of latency; 0 gives responsiveness and mismatch. | Code ([`components/viz/common/switches.cc`](https://chromium.googlesource.com/chromium/src/+/130ab80784123014a0e205433955f2a0450a85f4/components/viz/common/switches.cc), [`components/viz/service/surfaces`](https://chromium.googlesource.com/chromium/src/+/227bfbd618acce7469b9f5700725dc85dc498c1f/components/viz/service/surfaces)); the design doc itself I could not retrieve |
| **Chromium on Windows, window frame** | `DesktopWindowTreeHostWin::HandleWindowSizeChanging` avoids resizing the window until the swaps to it have finished; `HWNDMessageHandler` has a redraw lock that is disabled when DirectComposition is used with a child rendering window, because `WS_CLIPCHILDREN` on the parent should prevent glitched rendering. An older `CompositorResizeLock` held the compositor with a timeout until the renderer produced a frame at the new size (the pre-viz design). | Code and review threads (snapshots are old; they show the idea, not today's flow) |
| **Electron, macOS (Chromium M147 and later)** | Resizing a web contents view embeds the renderer's new surface with a **deadline of 0**, so the window's new frame reaches the screen with the page's previous frame. Pacing resizes and "render before reveal" only partly helped (out of step in 60 to 95% of frames with switches alone; up to 140 px behind when shrinking). A patched Electron that makes each resize **wait for the page's frame at the new size** gave 0 out-of-step frames in every direction; cost: the window resizes at most at the page's frame rate. Windows and Linux untested; no hung-page timeout is described. | Doc ([electron-resize-sync](https://github.com/lennondotw/electron-resize-sync)) |
| **Electron, general** | `setAutoResize` is gone with `WebContentsView`; apps listen for the window's `resize` and call `setBounds`, which runs after the window changed, so a frame or more of lag is expected. Workarounds: coalesce `setBounds`, set bounds in the main process rather than round-tripping the renderer, match the background colour, show a `capturePage()` snapshot during the resize. | Issue ([#22174](https://github.com/electron/electron/issues/22174), [#42201](https://github.com/electron/electron/issues/42201)) |
| **VS Code** | The integrated browser is a `WebContentsView` owned by the main process; the editor pane sends bounds over IPC. I found no measurement of its resize lag. Extension webviews are iframes inside the page, so they resize with normal layout. | Doc/code (its `integrated-browser` skill file); lag **Not found** |
| **WebView2** | Windowed hosting is the simple mode and has known white bands on live resize (the colour is painted inside the child windows, which the host cannot fix). **Visual (composition) hosting** puts the WebView in the host's own DirectComposition tree so bounds and visuals change in one `Commit`. `SetBoundsAndZoomFactor` helps when size and zoom change together. | Doc ([Windowed vs. Visual hosting](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/windowed-vs-visual-hosting)); Issue (Electrobun, WinUI) |
| **CEF** | Windowed: handle `WM_ERASEBKGND` on the parent so it does not paint over the child; resize the child once per size change. Off-screen: after `WasResized` the new-size `OnPaint` arrives asynchronously, never synchronously with the host's resize; batch `WasResized`; compare the buffer's size to the current view size and keep drawing the last frame, scaled or cropped, until a matching buffer arrives. | Issue ([#2733](https://bitbucket.org/chromiumembedded/cef/issues/2733/viz-osr-might-be-causing-some-graphic), [#2869](https://bitbucket.org/chromiumembedded/cef/issues/2869/osr-browser-size-and-offscreen-surface)); forum |
| **Win32, DWM** | `WS_CLIPCHILDREN` on the parent, `DeferWindowPos` to move children together, no background erase. Windows synchronises a window frame with the app's content only if it knows when the app finished drawing: private `user32` ordinals (`EnableResizeLayoutSynchronization`, `GetResizeDCompositionSynchronizationObject`) plus DWM's synchronised commit do it, Windows 11 only. Other projects ([RAD Debugger #1033](https://github.com/EpicGames/raddebugger/pull/1033)) use them; Microsoft documents neither. Flip-model swap chains need their buffers resized and a frame presented inside `WM_NCCALCSIZE` to avoid a stretched frame. | Issue/PR; the APIs are **private** |
| **Tauri / wry** | `set_bounds` on a child WebView2 window is parent-relative and undocumented; `set_auto_resize` plus manual bounds can re-apply cached ratios; the advice is one `set_bounds` per change, driven by a `ResizeObserver` on a placeholder, and a snapshot while overlapped. Resize is slower in Tauri than in plain wry (an open report). | Issue ([#1875](https://github.com/tauri-apps/wry/issues/1875), [#6322](https://github.com/tauri-apps/tauri/issues/6322)) |
| **Windows Terminal, other hybrid shells** | **Not found**: no primary source on how they hold a child's content to the frame. | Not found |

### 3.1 What the field converges on

1. **Lockstep is a compositor property.** Where frame and content do stay in step (Chromium's own browser, WebView2 composition hosting, the patched Electron), the parent's frame is held for the child's frame **inside one compositor or composition tree, with a deadline**. Nobody gets it by making a JavaScript layer wait for a separate native window.
2. **The cost is stated everywhere the same way:** the window resizes at most at the page's frame rate; with little to render it is as fast as before. This is the trade the operator described.
3. **A hung child needs a deadline.** Chromium's deadline is the mechanism; Electron's patch does not say what it does.
4. **Windowed child windows are the known weak case:** separate swap chains, composed by DWM, stretched old frame until the new one arrives, white bands.
5. **Cheap mitigations are universal:** coalesce bounds updates, set bounds from the host (not a renderer round trip), match the background colour, snapshot or hold the last frame.

---

## 4. Options

How each feels at 1, 3 and 6 panes and with a hung page is stated from the measurements in section 2 and the prior art; where it is a prediction, it says so.

### (a) Gate the chrome: the frontend holds the new layout until the browser reports a frame at the new size

- **How:** on each size change the page computes the new layout but keeps rendering the old one; it sends the rects, waits for each pane to report a painted frame at the new size (a CDP `requestAnimationFrame` plus `visualViewport` round trip, or a fork patch exposing "frame at the new LocalSurfaceId"), then releases the layout. A timeout (for example 100 ms) releases it anyway.
- **Benefit:** if it worked, the chrome and content would move together.
- **Cost and problems:**
  - The chrome is painted by the main window's own compositor and the OS frame moves anyway: the user would see the window frame ahead of both the old chrome and the browser, with an empty strip.
  - On current `main` the chrome is already the slowest layer (section 2.3); making it wait for the browser makes the slow thing slower.
  - There is no "painted at the new size" signal for windowed panes (Korp). A new one is a fork patch or an async CDP probe (ms per pane, per frame, from the main page).
  - Resize rate would be capped at the slowest pane's paint rate: **1 pane**, near today's rate; **3 panes**, the slowest of three; **6 panes**, the 10-06 GPU result (72% busy with nine panes) means a low and uneven rate. **Hung page:** the timeout fires every frame, so every resize waits the full timeout (10 fps at 100 ms), or the page is exempted after the first miss and the gate is off for it.
- **Verdict:** not recommended as the way to lockstep.

### (b) Same composition: host panes in the main window's own compositor

- **How:** Windows panes become CEF Views `BrowserView` overlays (what macOS and Linux do), so viz aggregates their surfaces into the window's frame. Raise the surface-synchronisation deadline for resize so the compositor holds the parent frame until the pane's surface for the new size arrives (Electron's patch is the existence proof; whether a switch or a fork patch is needed is what the spike finds out).
- **Benefit:** atomic by construction: one swap chain per window, bounds and content commit together, no wrapper HWND and no HWND subclassing; removes the per-pane swap chain GPU cost (10-06 cause 3).
- **Cost and risk:** the 2 to 3 day spike in the 10-06 analysis; CEF issue #3790 (an overlay view's renderer never initialising) was the April blocker; airspace is not solved by it (an overlay still draws above the app's page, so clipping needs a fork patch); focus and IME differ; opaque backgrounds only. Resize rate would be capped at the slowest pane's frame rate (the operator's stated trade). **Hung page:** relies on the deadline, so the cost is a stall of one deadline per frame unless the hung pane's surface is exempted.
- **Resize feel (predicted):** 1 pane, as stock; 3 panes, capped at the slowest; 6 panes, the cap is lower but panes no longer each present on their own swap chain. Needs the spike.
- **Verdict:** the only option that can give real lockstep. Spike first.

### (c) Cover the lag: hide the stretch or crop, then snap

- **How:** (i) paint each pane's wrapper and the exposed strip in the pane's background so a gap is not white or black; (ii) for a pane that fails a responsiveness probe at drag start, show its last good frame in the placeholder (the existing drag snapshot, `use-drag-snapshot.ts`) and hide the native window until the drag ends (the 10-06 option 5.5), per pane only.
- **Benefit:** cheap, no new signal needed for (i); (ii) removes the worst artefact (a hung page stretched across the pane). No change to the chrome's speed.
- **Cost:** a snapshot needs to be fresh; static content while dragging for the frozen pane; (ii) needs the probe.
- **Feel:** 1, 3 and 6 panes unchanged for healthy panes; **a hung page** stops stretching. This is the interim to pair with b's spike if section 8's run shows stretch or crop is what is seen.

### (d) Throttle resize events to the browser's frame rate

- **Already done in the sense that matters:** one batch per window per frame, newest wins, with host-side prediction (`pane-rect-batcher.ts`, `bounds.rs`). Throttling further would slow the chrome without making the browser's frame arrive sooner. The measured lag is not an event-rate problem (the 10-06 queue is gone).
- **Verdict:** nothing left to gain here.

### (e) Not listed in the task, but the strongest alternatives to (a)

- **Private DWM resize synchronisation** (the ordinals above): Windows 11 only, undocumented, could break with any update. Not recommended.
- **Composition hosting of the pane in our own DirectComposition tree** (the WebView2 pattern): needs windowless rendering and our own input routing; the 10-06 analysis rejected OSR as more per-frame cost than (b) for no gain.

---

## 5. Recommendation and plan

1. **Do not build (a).** The measured lag is the opposite of the premise on `main`, the chrome cannot hold the OS frame, and there is no signal to wait on.
2. **Run the missing measurement** (section 8) on a machine where the window can be in front, to learn whether the operator's complaint is the chrome trailing, stretch or crop of stale content, a busy or hung page, or the nine-pane GPU load.
3. **Spike (b) on Windows** with the same harness and decide on numbers: presents per frame (target one per window), GPU main thread load (target well under 50%), and the lag in frames. In the spike, also try the deadline.
4. **Interim, if the run shows stretch or crop:** (c) (i) now, (c) (ii) only for a pane that fails the probe.

### 5.1 Measurement plan, so the build can be proven

**Metric A (chrome against the browser):** frames the native pane box is ahead of or behind the chrome, right-most pane, from the strip capture: median, p90, worst.
**Metric B (content freshness):** frames the page's content is older than the pane box, from the bar width: median, p90, worst, and the share of drag frames not current.
**Metric C (cost):** resize rate achieved (distinct DOM positions per second) and GPU main-thread busy share.
**Matrix:** 1, 3, 6 and 9 panes; light, busy and one hung page; 10 and 30 px per frame; grow and shrink; also the modal sizing loop with real input on a machine where that is allowed.
**Pass bar (proposed):** A and B at most 0.5 frame at the median and 1 frame at p90 with 6 panes and a busy page; a hung pane does not stretch; the achieved resize rate with a hung pane stays above 30 per second; no regression with 1 pane.

---

## 6. Test instance, flags and what was not done

- **Build:** `task dev` (dev profile: `opt-level=1`, LTO off) from `ce93302`, started at below-normal priority. Window title `agent2-5-resize`, isolated home `.dev-home/5175`.
- **Flags:** none of `--disable-features=CalculateNativeWinOcclusion`, `--disable-backgrounding-occluded-windows` or `--disable-renderer-backgrounding` were set. The window was kept un-covered while captured, so occlusion throttling did not apply to these numbers. The leader (AgentY) asked for those flags in any future run with the window parked; they should be named in that run's report.
- **Foreground:** I raised the window to the top (`HWND_TOPMOST`) to capture it. AgentY asked me to stop; it was released and nothing is running. No further capture has been done.

## 7. Risks

- Stepped `SetWindowPos` input is not the OS modal sizing loop; DWM may behave differently for a real drag.
- Solid-colour pages are lighter than real pages; the busy page is a stand-in.
- The "native leads the chrome" result depends on `follow_window_resize` (Windows, `WM_SIZE`); if the chrome were faster, the sign would flip.
- The strip capture reads display frames at 59 Hz; sub-frame timing is not visible.
- Korp checked the host-side facts (child HWNDs, no painted-at-new-size signal, what the `set_rects` reply means) against the code, and has **not** verified the viz `deadline-to-synchronize-surfaces` behaviour or the CEF #3790 claim; those rest on the sources cited.
- (b) is a spike with a known earlier blocker (CEF #3790) and, if it needs a fork patch, a cost to the `agentmuxai/cef` fork.

## 8. Pending

1. **Stretch-aware matrix** (1, 3, 6 panes, busy page, hung page) with `analyze3.py`: written, not yet run beyond one pane. Needs the window in front for a few minutes.
2. **Nine real pages and the modal sizing loop,** the operator's actual case: needs a slot on a machine where the window may be in front.
3. **Korp to read this draft** (the host-side facts in sections 1 and 4 are theirs to correct).

---

## Sources

AgentMux code (`ce93302`): `crates/cef/src/browser_panes/bounds.rs`, `crates/cef/src/browser_pane/{wrapper,creation,creation_views}.rs`, `frontend/app/view/browser/pane-rect-batcher.ts`, `frontend/layout/lib/TileLayout.win32.tsx`. Earlier work: `docs/analysis/ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md`, `ANALYSIS_BROWSER_PANE_AIRSPACE_ARCHITECTURE_2026_05_30.md`, `docs/CEF_ARCHITECTURE.md`.

External (fetched 2026-11-03): [electron-resize-sync](https://github.com/lennondotw/electron-resize-sync); Electron issues [#22174](https://github.com/electron/electron/issues/22174) and [#42201](https://github.com/electron/electron/issues/42201) and the [WebContentsView migration note](https://www.electronjs.org/blog/migrate-to-webcontentsview); WebView2 [Windowed vs. Visual hosting](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/windowed-vs-visual-hosting); CEF issues [#2733](https://bitbucket.org/chromiumembedded/cef/issues/2733/viz-osr-might-be-causing-some-graphic) and [#2869](https://bitbucket.org/chromiumembedded/cef/issues/2869/osr-browser-size-and-offscreen-surface); Chromium source for [viz switches](https://chromium.googlesource.com/chromium/src/+/130ab80784123014a0e205433955f2a0450a85f4/components/viz/common/switches.cc) and [surface dependency tracking](https://chromium.googlesource.com/chromium/src/+/227bfbd618acce7469b9f5700725dc85dc498c1f/components/viz/service/surfaces) and the Windows [`HWNDMessageHandler`](https://chromium.googlesource.com/chromium/src/+/299155e5e37a77670b7969771e09e9a16b1f5612/ui/views/win/hwnd_message_handler.cc); [RAD Debugger PR #1033](https://github.com/EpicGames/raddebugger/pull/1033) (private DWM resize synchronisation); wry [#1875](https://github.com/tauri-apps/wry/issues/1875) and Tauri [#6322](https://github.com/tauri-apps/tauri/issues/6322). The Chromium viz "Surface Synchronization" design document and any Windows Terminal write-up were not found.

Agent: Agent2-5@narko
