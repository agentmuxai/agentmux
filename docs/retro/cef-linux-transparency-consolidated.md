# Linux Window Transparency — Consolidated Investigation

**Scope:** All investigation into `window:transparent=true` rendering on Linux/Wayland under CEF.  
**Status as of 2026-06-25:** Partial (gaps/borders transparent, promoted pane layers opaque). Root cause for the remaining opacity is **known** but not yet patched.  
**Source docs:** `cef-transparency-research-2026-05-10.md`, `cef-transparency-empirical-2026-05-11.md`, `cef-transparency-session-2-2026-05-11.md`, and memory `cef_transparency_root_cause.md`.

---

## What works

- Wayland surface is ARGB8888 (alpha channel allocated, `wl_surface_set_opaque_region` NOT called).
- Window border / tab-bar gap / body-only regions: desktop wallpaper bleeds through.
- Browser-side compositor: `has_transparent_background=1`, `will_fill_screen=0`. Fully patched.
- Frontend CSS: `--window-opacity`, `is-transparent` class, `:root` variable. Correct.
- CEF command-line flags: `background-color: 00000000`, `--disable-lcd-text` — gated on `window:transparent` as of commit `b50de537` (2026-06-25). Correct.
- `CefSettings.background_color = 0x00000000` and `BrowserSettings.background_color = 0x00000000` — correct.

## What does NOT work

**Promoted compositing layers (pane interiors) composite over opaque white instead of the desktop.**

Pixel measurements (2026-06-08, live app with green wallpaper):
- Gap/border region → `rgb(14, 77, 14)` ✓ green tint
- Pane interior (promoted layer) → `rgb(62, 62, 62)` ✗ no tint — opaque base

Varying `--window-opacity` 0.85→0.25 produced **zero visible change in pane interiors** — they composite against an opaque white base INSIDE the renderer, not the desktop.

---

## Root cause (CONFIRMED 2026-06-08, not yet fixed)

`libcef/renderer/` **never calls** `WebViewImpl::SetBaseBackgroundColorOverrideTransparent(true)`.

In Blink, `page_base_background_color_` defaults to `SK_ColorWHITE`. Every composited (promoted) layer that paints a page background paints this white as the clear color. The standard Chromium mechanism to flip it is `WebViewImpl::SetBaseBackgroundColorOverrideTransparent(true)` (web_view_impl.cc:3555). CEF's renderer process never calls it — confirmed by `grep -rn SetBaseBackgroundColor libcef/renderer/` → nothing.

All browser-side patches (RWHView, WebContents, ui::Compositor) correctly reach `cc::LayerTreeHost.has_transparent_background=1`. The IPC chain fires correctly. But the renderer's Blink base color stays opaque white, so promoted layers paint opaque.

This is a **libcef source patch** required.

### The patch (not yet implemented)

In `libcef/renderer/` — in the render-frame or browser-impl initialization path (and across renderer process swaps via a `WebContentsObserver`-equivalent):

```cpp
// If CefSettings.background_color is transparent, tell Blink to use
// a transparent page base color instead of the SK_ColorWHITE default.
// Without this, promoted compositing layers paint opaque white.
if (CefContext::Get()->GetBackgroundColor(nullptr, STATE_ENABLED) == SK_ColorTRANSPARENT) {
    GetWebView()->SetBaseBackgroundColorOverrideTransparent(true);
    // or: GetWebView()->SetPageBaseBackgroundColor(SK_ColorTRANSPARENT);
}
```

This requires either adding `SetBaseBackgroundColorOverrideTransparent` to the public `blink::WebView` interface (small blink patch), or calling it via `blink_glue` (which already reaches into blink internals). Must re-apply across renderer process swaps.

---

## Dead ends — approaches that did NOT fix it

These are documented to prevent re-investigation.

### 1. Wayland-side protocol errors (2026-05-11)
`OnTrancheFlags not implemented`, `GpuControl.CreateCommandBuffer failed` — appeared threatening but were red herrings. WAYLAND_DEBUG confirmed `wl_buffer` IS ARGB8888; the opaque pixels come from the renderer, not the Wayland surface plumbing.

### 2. `wl_surface_set_opaque_region` suppression (2026-05-11)
Confirming this isn't called was necessary but insufficient. The wl_buffer is ARGB; the problem is the pixels inside it are opaque white.

### 3. `contents_opaque=false` in `content_layer_client_impl.cc` (2026-05-11)
Changed `!= kTransparent` → `isOpaque()` so layers with alpha < 1 aren't incorrectly marked opaque. All layers showed `final_contents_opaque=0`. **No visual change.**

### 4. `UpdateBaseBackgroundColor` re-push to FrameWidget (2026-05-11)
Added re-push so cc::LayerTreeHost.background_color gets the alpha-aware value. LTI log confirmed `commit.bg=(0.133, 0.133, 0.133, 0.451)` and `has_transparent_background=1`. **No visual change at pane interiors.** The browser-side compositor is correctly transparent; the problem is renderer-side.

### 5. `--disable-lcd-text` switch (2026-05-11 session 2)
Tried before the renderer root cause was understood. CLC log confirmed all layers `contents_opaque=0`. **No visual change.**

### 6. Various brute-force rasterization patches (2026-05-11 session 3)
`SoftwareRenderer::DrawTileQuad` skip, `pending_layer.cc::UsesSolidColorLayer`, `solid_color_layer_impl.cc::AppendQuads` clamp, "sticky transparency" latch, `tile_manager.cc`, `raster_source.cc`, `picture_layer_impl.cc`, `aura/window.cc` defaults. Some caused rendering regressions; those that didn't produced no visible change or partial results at best.

### 7. Chad Nelson's Views-side patch alone (2026-05-10 → 05-11)
The existing CEF PR `b921ffe18` (kColorPrimaryBackground in CefContext) + the deferred `SetBackgroundColor(SK_ColorTRANSPARENT)` on the top-level widget. These are correct browser-side but do not reach the renderer's `page_base_background_color_`. Partial transparency only.

### 8. CEF window_view.cc `is_translucent` gate (2026-06-24, PR #3 on agentmuxai/cef)
Gated `is_translucent = is_frameless_ && color == SK_ColorTRANSPARENT` so non-frameless windows don't go translucent. Correct and shipped in `cef-linux-x86_64-148.0.20-2`. Does not affect the pane-interior opacity — the Views widget opacity is already correct; the problem is the renderer-side Blink base color.

### 9. App.rs command-line flag gating (2026-06-25, commit `b50de537`)
Gated `background-color: 00000000` and `--disable-lcd-text` on `window:transparent` setting. Valid quality improvement (opaque windows no longer penalized). **Does not make `window:transparent=true` actually produce transparent pane interiors.** The root cause is renderer-side, not command-line flags.

---

## CEF patches in the current build (`agentmuxai/cef` `agentmux/7778-drag-rightclick-and-transparency` HEAD `c87bca497`)

All present and verified in the `148.0.20-2` release:

| Patch | Effect | Status |
|---|---|---|
| `BeginWindowDrag` | Left-click title-bar drag via Wayland xdg_toplevel.move | ✓ Working |
| Right-click HTCAPTION passthrough | Context menu on drag region | ✓ Working |
| `CefWindowView::CreateWidget` `is_translucent` gate | Views widget opacity correctly gated on frameless+transparent | ✓ Working |
| Deferred `SetBackgroundColor(transparent)` on top-level widget | browser-side ui::Compositor transparent | ✓ Working |
| WebContentsObserver `TransparencyApplyOnRenderReady` | RWHView + direct SetBackgroundOpaque(false) | ✓ Working |
| **`SetBaseBackgroundColorOverrideTransparent` in renderer** | **Blink base color override** | **❌ NOT YET IMPLEMENTED** |

---

## Next concrete step

Implement the renderer-side patch in `libcef/renderer/` as described above. This is the one remaining blocker for full pane-interior transparency. No other approach has produced full transparency without regressions.

**Estimated effort:** 1-2 days (source change, OOM-safe rebuild ~30 min incremental, verify, publish to `agentmuxai/cef` releases as `148.0.20-3`).

**Note:** This is a `libcef.so` source patch — it requires a rebuild. It cannot be fixed at the AgentMux app.rs / frontend level. An AI agent without access to the `~/cef-build` tree and the ability to run a multi-hour build cannot complete this task unattended.
