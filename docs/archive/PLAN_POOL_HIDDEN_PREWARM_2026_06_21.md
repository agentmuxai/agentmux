# Implementation Plan: hidden-pre-warm window pool (fix Windows blank-on-promote)

**Date:** 2026-06-21
**Area:** `agentmux-cef` — window pool (`src/commands/window_pool.rs`, `src/ui_tasks.rs`, window creation)
**Goal:** keep the new-window/tear-off pool, but make pool-promoted windows **paint correctly on Windows**.
**Backed by:** `docs/research/RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21.md`
**Prior point-fixes (orthogonal, already merged):** #1650 (default layout), #1652 (on-screen clamp).

---

## 0. RESOLUTION (what actually fixed it — added after implementation)

The research's *diagnosis* was right (a Windows compositor visibility-state
desync; macOS works because it shows via CEF Views), but the *plan's* "hidden
pre-warm" approach was **not** what fixed it. The actual fix, verified on HiDPI:

1. **Render (load-bearing):** the Windows promote positioned the raw HWND via
   Win32 but **never drove the CEF Views `Window.show()`** that flips the
   browser's compositor visibility — so the renderer's frame was evicted and the
   window painted blank. Fix: cache the CEF Views `Window` at `on_window_created`
   (a global `Mutex`, because `browser_view.window()` is `None` for pool windows
   post-load and the promote runs on the IPC thread), then **post a UI task** that
   runs `set_bounds()` + `show()` — the exact macOS sequence, on the UI thread.
   Ruled out first (all no-ops/wrong-layer): HWND show-order; `was_hidden()`/
   `was_resized()` host hints; a direct `show()` that couldn't reach the Window;
   a thread-local cache read on the wrong thread.
2. **Window identity (drag/close/min/max):** promoted pool windows never landed
   in `state.window_hwnds`, so `resolve_window_hwnd` fell back to the main window
   and the new window's chrome acted on the original. Fix: register the promoted
   outer HWND under its label at promote.
3. **Taskbar name:** the host exe is CEF's `bootstrap.exe` ("CEF Bootstrap
   application" FileDescription). Fix: `inject-exe-icon.sh` now also stamps
   `FileDescription`/`ProductName` = AgentMux via rcedit.

The pool is fully retained (no cold-path fallback needed).

---

## 1. Problem in one line

Pool windows are created **visible-but-off-screen** at `(-32000,-32000)`. On Windows that puts the aura `Window` in the **"already visible"** state, so the promotion's move+resize generates no **hidden→visible transition**, and Chromium's compositor/occlusion state never re-syncs → the on-screen window is **blank** despite a correct DOM (research §1a, cef#3638). macOS is unaffected (research §3).

## 2. Core idea (the Electron/VS Code model)

Stop pre-warming **shown-off-screen**. Pre-warm **genuinely hidden** (never shown), then do a real **hidden→visible transition** on promote. This is the CEF analog of `BrowserWindow({show:false})` → `ready-to-show` → `win.show()` (research §2). We keep the main pool benefit — the CEF window + renderer process are already spawned and the (empty, pool-mode) page is loaded, which is the ~3 s saving. We do **not** rely on a *pre-painted* surface (Windows may not paint a hidden window; research §5.3), and we don't need to: pool windows render an empty body in pool mode and only bootstrap the workspace **at promote time** anyway, so nothing is pre-painted today either.

## 3. Current code (what changes)

- `spawn_pool_window` (`window_pool.rs:193`) → `crate::ui_tasks::post_create_window(state, url, label, POOL_OFFSCREEN_X, POOL_OFFSCREEN_Y, w, h, frameless=true)` (`:348`). The window is created **shown** (comment `:345` "visible (frameless) but well outside any monitor bounds").
- `promote_pool_window` (`window_pool.rs:547`, Windows) → `set_taskbar_hidden(false)` + `SetWindowPos(HWND_TOP, rect)` + `ShowWindow(SW_SHOW)` + (post-#1652) clamp + re-assert.
- Pool window post-create handling: `on_after_created` / `register_pool_window` (`window_pool.rs`), `set_taskbar_hidden` keeps it out of the taskbar.

## 4. Plan — staged, repro-gated (the research flagged real uncertainty)

The research could not pin a single mechanism and noted #3638 may be patched in our CEF and that hidden windows may not pre-paint on Windows. So **Stage 0 is a decisive spike** before the full change.

### Stage 0 — Spike: create pool windows hidden, measure (½ day)
Smallest change that tests the hypothesis:
1. Add a `show: bool` parameter (or a `post_create_window_hidden` variant) to the pool window-creation path so the CEF window/native HWND is created **without `WS_VISIBLE`** (or `ShowWindow(SW_HIDE)` immediately at `on_after_created` for `window-pool-*` labels) and is **not** positioned at `-32000` while visible. The window can sit hidden at any size (use `POOL_WIDTH×POOL_HEIGHT`).
2. In `promote_pool_window` (Windows), keep the existing sequence — it already ends in `ShowWindow(SW_SHOW)`. Because the window was never shown, this `SW_SHOW` is now the **genuine first show** (hidden→visible). Add a `CefBrowserHost::WasResized()` (or a 1px size jiggle) right after show as belt-and-suspenders.
3. **Build + verify** (see §5). Decision gate:
   - **Blank fixed** → proceed to Stage 1 (polish).
   - **Still blank** → the dominant mechanism isn't the "already visible" path; go to Stage 2 (forced-resync or layered pre-warm), using `about:gpu` + `--enable-logging` occlusion logs to identify which of occlusion-tracker / FrameEvictor is responsible (research §5.1).

### Stage 1 — Productionize the hidden pre-warm (if Stage 0 works)
- Make "hidden" the real pool-window state end to end: never `SW_SHOW` until promote; ensure `set_taskbar_hidden`/`WS_EX_TOOLWINDOW` still applies while hidden (a hidden window has no taskbar entry anyway, but keep the style correct for the post-show taskbar reveal).
- Promote sequence (authoritative, single place): `SetWindowPos`(final clamped rect, real size delta) → `set_taskbar_hidden(false)` → `ShowWindow(SW_SHOW)` → `UpdateWindow` → `WasResized()`. Keep the #1652 work-area clamp + post-show re-assert + off-screen telemetry.
- Drop `POOL_OFFSCREEN_X/Y` usage for the new-window/tear-off pool (no longer needed; a hidden window needs no off-screen parking). Keep the constant only if the pane-pool path still relies on it; otherwise remove.
- Refill (`spawn_pool_window` after each promote) creates the replacement **hidden** too.
- Confirm **tear-off** still works: tear-off promotes the same window; a hidden→shown window must still land under the cursor and run the SC_MOVE drag handshake. Verify the drag handshake doesn't depend on the window being pre-shown.

### Stage 2 — Fallback (only if hidden pre-warm doesn't paint)
If a genuinely-hidden window still won't paint on first show in our CEF build:
- **2a. Forced promotion-time resync:** after show, a real resize (e.g. final-1px then final) to force `WM_SIZE`, and/or a `SW_HIDE`→`SW_SHOW` toggle (research §4c, electron#27353 "paints after move/resize"). Cheap, ugly, effective.
- **2b. Layered/transparent on-screen pre-warm:** create the pool window **on-screen but fully transparent** (`WS_EX_LAYERED`, alpha 0) so it is neither off-screen-visible nor hidden — it paints (not occluded) and reveal = set alpha 255 + reposition. More complex; only if pre-painted pixels are actually required. (research §5.3)
- **2c. GPU/feature flags** as mitigation layer: `--disable-features=CalculateNativeWinOcclusion` (already set) **plus** `--disable-backgrounding-occluded-windows --disable-renderer-backgrounding`. Not a standalone fix (refuted 0-3 individually) but may stack with 2a.

## 5. Verification

**Empirical repro harness (build once, reuse each stage):**
- Launch the build; drive N "Open another window" via CDP (`window.api.openNewWindow()`); for each promoted window capture:
  - position/size + `blocks`/`tiles` (already scripted) — confirms DOM + placement, AND
  - **`Page.captureScreenshot` byte-size per pixel vs a known-blank pool spare and the known-good Starter** — distinguishes "DOM populated but surface blank" from "actually painted." (DOM-count alone is NOT sufficient — that's what masked this bug.)
- Pass criterion: every promoted window's screenshot has content-level entropy comparable to Starter (not to a blank spare), AND user-visual confirmation.
- Diagnostics if still blank: `about:gpu`, `--enable-logging=stderr --vmodule=native_window_occlusion*=1`, toggle one lever at a time.

**Regression:** tear-off a tab (window follows cursor, lands correctly, paints); multi-monitor + 125%/100% mixed DPI; pool exhaustion → cold path still works.

## 6. Risks & rollback
- **Risk:** hidden window doesn't pre-paint → first show has a brief paint-in (not a blank, just a frame later). Acceptable; if not, Stage 2b.
- **Risk:** tear-off drag handshake assumed a pre-shown window. Mitigation: verify in Stage 1; the SC_MOVE handshake runs after show in both cases.
- **Risk:** `WasResized()`/windowed-mode CEF host calls may be no-ops (some are OSR-only — `Invalidate(PET_VIEW)` is OSR-only, do NOT use). Rely on Win32 `ShowWindow`/`SetWindowPos`/`WM_SIZE` as the load-bearing mechanism.
- **Rollback:** the change is gated to pool window creation + promote (Windows `#cfg`). Reverting restores current behavior. If Stage 0/2 all fail under time pressure, the documented stopgap is to route `open_new_window` to the **cold path** (no pool) — correct, ~3 s slower — while keeping the pool for tear-off only.

## 7. Done =
- Spike result recorded (which stage fixed it) + the screenshot-based pass for ≥6 consecutive opens, 0 blank, on HiDPI.
- Tear-off + multi-monitor regressions pass.
- Research + this plan referenced in the PR; a short retro on the root cause (visibility-state desync, not GPU).
