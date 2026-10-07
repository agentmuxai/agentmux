# Handoff: window-resize performance, and browser panes as CEF Views overlays

**Date:** 2026-10-07
**Author:** Agent4
**Status:** resize performance work shipped; the browser-pane Views migration is **shelved at the spike stage**, proven feasible on Windows, with next steps below.
**Tracking:** agentmuxai/agentmux#4430.

The detailed analysis is in two documents:
- `docs/analysis/ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md` (§1–12), about the page side.
- `docs/analysis/ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md` (§1–8 on `main`; §9 and §9.1 on the spike branch), about browser panes.

This file is the map: what's done, what's parked and where, and what to do next.

---

## 1. Shipped (on `main`)

| PR | What |
|---|---|
| #4396 | Hidden window tabs skip layout while the window resizes (`content-visibility: hidden`), then relay out one per idle period. The tile layout takes its size from the ResizeObserver instead of a forced read. |
| #4398 | Host: `windowresize:tick` only while Shift is held. One trailing writer replaces a thread per position report. WinEvent logging moved to `trace!`. |
| #4399 | Terminals refit on every frame of a drag (long buffers through a ~100 ms column throttle), with the PTY told on settle. The System Info chart follows the drag. |
| #4404 | The tile layout no longer flips its `animate` class during a resize (each flip restyled the whole tab). Plot's per-chart `<style>` is gone. Chart redraws are throttled with stretching in between. Pane tab strips measure on the next frame. |
| #4406 | Removed Tailwind's unused `.container` and its five width breakpoints. Each crossing invalidated fonts and relaid out the tab. |
| #4408 | Terminals take turns refitting, at most two per frame. |
| #4415 | The agent list measures resized rows in one pass with one `RowsMeasured`, instead of a forced layout and a prefix-sum rebuild per row. |
| #4420 | Browser panes: rects go in one batch per window with at most one in flight, applied by the host in one `DeferWindowPos`. Console logs go in batches. Favicons are sent only when they change, and each pane logs only its own events. That logging had been filling the IPC connections. |

Measured results are in each PR and in the two analyses. In summary:
- **Window drags** on agent-pane and terminal tabs run at about 60 fps in synthetic tests; the page itself trails the edge by one frame, which is the floor.
- **Browser panes** went from about 540 ms behind during a drag to one or two frames, all moving together.

## 2. Parked: browser panes as CEF Views overlays on Windows

**Why it's worth doing:** compositing. With panes as native child HWNDs, the GPU process presents about ten separate outputs for a window with nine panes, and its main thread is about 70% busy during a drag. As Views overlays it presents one output at 60 fps and is about 52% busy. The host's own move is only a little cheaper (p50 9.4 against 12.5 ms). See analysis §9.

**Where it is:**
- **App code:** branch `agent4/pane-views-spike` in `agentmuxai/agentmux`. **Not for merge**; it's a spike.
  - `crates/cef/src/browser_pane/views_spike.rs`: create, batched move, shape and close for Views panes on Windows.
  - Guards in `callbacks.rs`, `clip.rs`, `close.rs`, `navigation.rs` and `bounds.rs`. For a Views pane, `host.window_handle()` is the **main window's** HWND, so every Windows path that acts on a pane's own HWND must skip it.
  - `display.rs`: a pane must never set a window title (see §3.1).
- **CEF patch:** branch `agentmux/overlay-shape-8037` (`cae35eefd`) on `agentmuxai/cef`. It adds `CefOverlayController::SetShape(const std::vector<CefRect>&)` (`added=15400`).
  - It calls `views::Widget::SetShape`, which on Aura becomes a layer alpha shape and clips the web content too.
  - It installs an `aura::WindowTargeter` whose `GetExtraHitTestShapeRects` returns the same shape, so events outside it reach the app page.
  - About 60 lines.
- **Switches** (the spike's code paths don't run unless set):
  - `AGENTMUX_PANE_VIEWS=1`: new panes are Views overlays.
  - `AGENTMUX_PANE_VIEWS_SHAPE=1`: call `set_shape`. **Only with a libcef built with the patch**, since the spike reads the slot past the stock struct.

**What's proven:**
- Panes render, move and close as Views overlays on Windows.
- A DOM menu (the top bar's "more" dropdown) draws over a pane, with the pane clipped exactly around it; closing the menu restores the full pane.

**Not yet verified:**
- Clicking menu items that sit over a pane. The hit-test half of the patch is in place but untested by hand.
- Keyboard, IME, focus, popups and DevTools in a Views pane on Windows.
- Why the two paths lay out the same page differently at DPR 1.25: only the native path shows horizontal scrollbars.

### 2.1 To resume

1. **Build libcef with the patch** (about 13 minutes incremental in an existing `~/cef-build` 154 tree with `out/Release_GN_154`):
   - Apply the fork branch's diff to `chromium/src/cef` (the build compiles from that copy, not `chromium_git/cef`).
   - Run `python tools/translator.py --root-dir .` and then `python tools/version_manager.py -u` in that directory.
   - Run `ninja -C out/Release_GN_154 libcef` with `build-154.ps1`'s environment.
2. **Run a dev build on it:**
   ```
   AGENTMUX_PANE_VIEWS=1 AGENTMUX_PANE_VIEWS_SHAPE=1 \
   AGENTMUX_ALLOW_UNVERIFIED_CEF=1 \
   AGENTMUX_CEF_RUNTIME_DIR_WINDOWS=$HOME/cef-build/chromium_git/chromium/src/out/Release_GN_154 \
   task dev
   ```
   - `AGENTMUX_ALLOW_UNVERIFIED_CEF=1` is the documented opt-out for a local CEF build; the runtime guard otherwise rejects an unpinned libcef. That tree's `args.gn` has `enable_backup_ref_ptr_instance_tracer=false`, which is what the guard protects.
   - Dev data is kept per branch, so recreate a tab of browser panes once on the spike branch.

**State of the shared `~/cef-build` tree at handoff:**
- **Sources:** `chromium/src/cef` and `chromium_git/cef` are back to the fork's `8037` commit, with no patch applied.
- **`out/Release_GN_154/libcef.dll`:** still the **patched build**. The patch is additive, and the next build of that tree replaces it. It wasn't rebuilt at handoff because the machine's page file was nearly full and a thin-LTO relink is memory-heavy.
- **The published runtime:** what `task dev` downloads into `out/Release_GN_x64` was never touched.

### 2.2 Proposed migration plan

1. **Land the CEF patch:** PR `agentmux/overlay-shape-8037` into `agentmuxai/cef` `8037`, then cut a Windows runtime release with it and update the runtime pin (`scripts/cef-build/windows-runtime-pin.sh`).
2. **Bindings:** add the `set_shape` slot to `_cef_overlay_controller_t` in `agentmuxai/cef-rs` (all platform bindings), following the `begin_window_drag` precedent, so the app calls it through the binding and not through the spike's pointer read.
3. **Per-pane zoom** (§3.2), in the same CEF release if possible.
4. **Make Views panes a setting on Windows**, porting the spike into the real pane manager:
   - real close, with the deferred controller destroy that Linux and macOS use;
   - focus;
   - clipping through `SetShape`, replacing `SetWindowRgn`.
5. **Verify** input, IME, popups and DevTools by hand.
6. **Make it the default.** Then delete the wrapper HWND, the HWND subclassing (`hwnd.rs`), `SetWindowRgn` clipping and the drag-snapshot workaround where `SetShape` covers it.
7. **Consider `SetShape` for Linux,** whose whole-pane hide plus freeze frame it could replace, and evaluate macOS, which takes a different native path for `Widget::SetShape`.

## 3. Findings to act on, independent of the migration

### 3.1 A browser pane's title can become the main window's title (Linux and macOS today)

`AgentMuxHandler::on_title_change` (`client/display.rs`) calls `window.set_title(title)` on the window that owns the browser's `BrowserView`. For a Views-hosted pane, which is every pane on Linux and macOS today, that window is the main window, so a page's title can replace the window title. The fix is to skip the window-title update when `self.is_browser_pane`. The spike branch has the change; it should land on `main` on its own. This was found by reading the code and isn't reproduced on Linux or macOS yet.

### 3.2 Per-pane zoom only exists on the Windows native path

Ctrl+wheel per-pane zoom works only because the Windows pane HWND subclass intercepts the wheel and the host applies CSS `zoom` per pane (`browser_pane/hwnd.rs`, `browser_panes/zoom.rs`). On Views panes (the spike, and very likely Linux and macOS today) the wheel reaches Chromium's native zoom, which is keyed by host, so every pane on the same site zooms together. The operator saw this in the spike.

**Recommended fix:** CEF attaches a `zoom::ZoomController` to every browser. A small fork patch can put pane browsers in `ZOOM_MODE_ISOLATED`, which is Chrome's per-tab zoom, so native zoom becomes per pane on every platform: Ctrl+wheel, Ctrl +/− and pinch. That retires both the wheel interception and the CSS-injection zoom.

### 3.3 Other notes

- **macOS `SetPaneBoundsViewsTask`** posts a 50 ms "reaffirm" that re-applies a stale rect during a live drag (`ui_tasks/pane_geometry.rs`). With batching it should reaffirm only the newest rect, or none while resizing.
- **The host's move of native panes** costs about 2 ms per pane inside Chromium's child-HWND resize, about 20 ms for nine. Views overlays are the way past it (§2).
- **Hidden-tab panes** (rect 0×0) keep live renderers. Throttling or suspending them (`WasHidden`) would cut memory and CPU on windows with many tabs of panes. The test machine's page file was nearly full during this work.

## 4. Measurement tools

The scripts are in Agent4's workspace (`~/.agentmux/agents/agent4-0831d/`), not in the repo:
- **`resizesteps.ps1`:** stepped `SetWindowPos` drag.
- **`frameprobe.mjs` and `traceprobe.mjs`:** rAF and long-animation-frame recording, and a Chromium trace of all processes.
- **`phases.mjs`, `busyframes.mjs`, `gpustats.mjs`:** frame-phase breakdown, and GPU main-thread busy share and present counts.
- **`edgeprobe2.ps1` and `dragshots.ps1`:** `PrintWindow` captures of the dev window, which work even when it's covered, for measuring how far the page or panes trail the edge.
- **`panebench.sh`:** the browser-pane benchmark (batch latency, GPU stats, end-of-drag capture).
- **`restart-dev.ps1` and `stop-dev.ps1`:** restart or stop Agent4's own `task dev` safely, optionally with environment switches.

Two lessons from using them:
- **Never measure the dev window with screen captures:** another window may be on top. `PrintWindow` captures the window itself.
- **Trace event sums by name double-count:** `Paint` nests inside `Paint`, and forced layouts nest inside ResizeObserver callbacks. Break frames down by Blink's top-level lifecycle phases instead.
