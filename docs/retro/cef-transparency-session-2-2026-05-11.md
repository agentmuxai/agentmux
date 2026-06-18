# CEF Wayland transparency — session 2 deep dive

**Date:** 2026-05-11 (afternoon session, 10:00–11:45 PDT)
**Status:** Significant progress — body-only/gap regions now bleed wallpaper through. Multi-layer pane interiors STILL render opaque-white-backed despite the renderer's `LayerTreeHost::background_color` correctly being transparent. Root cause for the remaining opaque-pane case identified but not yet fixed: a Chromium-internal rasterization path applies an opaque clear color to layers when `contents_opaque=false` does not propagate to `requires_clear=true`.

**Branches:**
- CEF fork: `a5af/cef` `agentmux/7680-drag-rightclick-and-transparency` (HEAD `3e041ad2f`)
- AgentMux: `agentu/cef-transparency` (HEAD `60d7c1fd`)

---

## TL;DR

After Chad Nelson's transparency patch in CEF (`b921ffe18`), CEF can theoretically support translucent windows on Linux/Wayland. The patch covers:

1. `CefSettings.background_color` alpha=0 → CEF context flips `is_transparent=true`
2. `CefBrowserSettings.background_color` alpha=0 → BrowserView's `default_background_color_` becomes `SK_ColorTRANSPARENT`
3. WindowDelegate `is_frameless()=true` → views::Widget initialized with `kTranslucent` opacity
4. `WindowOpacity::kTranslucent` → `PlatformWindowOpacity::kTranslucentWindow` → Wayland `IsOpaqueWindow()=false` → no `wl_surface_set_opaque_region` call → wl_buffer is ARGB8888 capable

For AgentMux on GNOME Mutter/Wayland with software compositing (GPU/Vulkan/WebGL blocklisted on this machine), all four of the above ARE in place. But we still observed opaque pane interiors.

This session walked the full pipeline with `LOG(WARNING)` diagnostics at every layer until we found the missing pieces:

1. **CefWindowImpl::SetBackgroundColor for top-level windows was never being called.** Existing `window_view.cc:738` modal-only branch ran with `widget_=null` and silently dropped the call. Browser-side `ui::Compositor` stayed at default opaque white. (FIXED)
2. **`--window-opacity` CSS variable was being set on `body`, not `:root`.** Theme.scss declares `--main-bg-color: rgba(34, 34, 34, var(--window-opacity, 1))` on `:root`. CSS substitutes var() at the element where the custom property is COMPUTED, so the substitution happens at `:root` using `:root`'s value of `--window-opacity`. Descendants inherit the already-substituted value. Setting `--window-opacity` only on body left the computed `--main-bg-color` at `rgba(34, 34, 34, 1)` for all 30+ panes. (FIXED — moved to documentElement)
3. **Renderer's `LayerTreeHost::background_color` stays opaque.** `SetBackgroundOpaque(false)` IPC properly flows from browser to blink, fires `SetBaseBackgroundColorOverrideTransparent(true)`, which calls `UpdateBaseBackgroundColor()` — but that function never re-pushes the new BackgroundColor through to `FrameWidget::SetBackgroundColor()`. The layer tree's clear color stays opaque white. Pre-fix `LayerTreeHostImpl::CalculateRenderPasses` saw `has_transparent_background=0 will_fill_screen=1`. Patched `UpdateBaseBackgroundColor` to re-push BackgroundColor() to the FrameWidget — now sees `has_transparent_background=1 will_fill_screen=0`. (FIXED)
4. **Even with all of the above, multi-layer pane interiors still render opaque-white-backed.** Pixel sampling over a green wallpaper:
   - Window border / gap regions: `rgb(14, 77, 14)` ✓ green tint (transparent)
   - Pane interior (body + block + agent-view layers stacked): `rgb(62, 62, 62)` ✗ neutral gray (opaque)
   - Mathematical analysis matches "0.5 over opaque white" not "0.5 over green wallpaper"

---

## Commits

### CEF fork (`a5af/cef`)

`3e041ad2f` — **views: deferred top-level transparent bg + observer cleanup**
- `CefWindowImpl::CreateWidget` — after `widget_` is assigned (`widget_ = root_view()->GetWidget()`), check global `CefSettings.background_color`; if transparent, call `SetBackgroundColor(SK_ColorTRANSPARENT)` so the browser-side `ui::Compositor`'s `cc::LayerTreeHost.background_color` becomes transparent. The existing `window_view.cc:738` call is inside `if (host_widget)` (modals only) and even there fires before `widget_` exists, silently dropping.
- `window_view.cc` — mirror `else if (is_translucent)` after the modal branch for top-level windows (kept for parity though the `CreateWidget` call is what takes effect).
- `browser_view_impl.cc` `TransparencyApplyOnRenderReady` — add a direct `host_->owner_delegate()->SetBackgroundOpaque(false)` call alongside `view->SetBackgroundColor(SK_ColorTRANSPARENT)`. The latter early-returns when the cached color matches, so the IPC may be suppressed after `SetDefaults` set the default to transparent.

Earlier commits already in the branch: `95ade0bad` (keep observer alive across renderer swaps), `6e0e93edb` (install WebContentsObserver), `2f69aabc5` (RWHView::SetBackgroundColor), `68e0dc668` (transparency cascade — propagate to WebContents).

### Chromium-side patches (committed in the chromium tree under cef/, NOT yet upstreamed)

These live in `~/cef-build/chromium_git/chromium/src/` and survive locally because the build pipeline regenerates `cef_api_versions.json` etc. They need to be ported to the CEF patch system (`cef/patch/patches/`) for cross-machine builds:

1. `third_party/blink/renderer/core/exported/web_view_impl.cc` `UpdateBaseBackgroundColor` — added a re-push to FrameWidget so SetBackgroundOpaque-induced base-color overrides actually reach the LayerTreeHost.
2. `third_party/blink/renderer/platform/graphics/compositing/content_layer_client_impl.cc` `contents_opaque` check — changed `!= SkColors::kTransparent` to `isOpaque()`. The original check incorrectly treated layers with alpha=0.45 backgrounds as opaque (alpha != 0 is not the same as alpha == 1).

### AgentMux (`agentu/cef-transparency`)

`60d7c1fd` — **fix(transparency): :root --window-opacity + disable-lcd-text**
- `frontend/app/app.tsx` `AppSettingsUpdater` — set `--window-opacity` on `documentElement`, not `body`. See the CSS-substitution-at-compute-time analysis above.
- `agentmux-cef/src/app.rs` — pass `--disable-lcd-text` Chromium switch. LCD subpixel AA requires opaque backgrounds and Chromium force-sets `contents_opaque=true` on layers with LCD text.

---

## Diagnostic trail

The diagnostic walkthrough (logs in `chrome_debug.log` for renderer-side, `logs/cef-debug.log` for browser-side):

### Browser side (PID = main agentmux process)

After deferred-SetBackgroundColor patch:
```
[AGENTMUX-TRANS-WIN-DEFERRED] applying transparent bg to top-level widget
[AGENTMUX-TRANS-WIN] SetBackgroundColor color=0 alpha=0 widget=non-null compositor=non-null
[AGENTMUX-TRANS-WIN] Compositor::SetBackgroundColor called
[AGENTMUX-TRANS-CC] CalculateRenderPasses: bg=(0,0,0,0) has_transparent_background=1 will_fill_screen=0
```

Browser-side compositor is fully transparent. ✓

### Renderer side (PIDs = forked zygote processes)

After WebContentsObserver-fires-RWHView::SetBackgroundColor + chromium UpdateBaseBackgroundColor patch:
```
[AGENTMUX-TRANS] RenderFrameCreated rfh=non-null primary=1
[AGENTMUX-TRANS] ApplyToCurrentRWHView: view=non-null
[AGENTMUX-TRANS] host_impl=non-null owner_delegate=non-null
[AGENTMUX-TRANS] DIRECT SetBackgroundOpaque(false) called
[AGENTMUX-TRANS-BLINK] UpdateBaseBackgroundColor: override_to_transparent=1 BackgroundColor=0 bg_alpha=0
[AGENTMUX-TRANS-BLINK] SetBackgroundColor called on FrameWidget
[AGENTMUX-TRANS-FW] SetBackgroundColor entry color=0 alpha=0 composite=1
[AGENTMUX-TRANS-FW] LayerTreeHost=non-null is_for_subframe=0
[AGENTMUX-TRANS-FW] set_background_color done, new=0
[AGENTMUX-TRANS-LTI] PullPropertiesFromCommitState: commit.bg=(0.133333, 0.133333, 0.133333, 0.45098)
[AGENTMUX-TRANS-CC] CalculateRenderPasses: bg=(0.133333, 0.133333, 0.133333, 0.45098) has_transparent_background=1 will_fill_screen=0
```

The chain reaches `LayerTreeHostImpl` with `has_transparent_background=true` and `will_fill_screen=false`. The "screen-fill" opaque quad is NOT being appended. ✓

### Per-layer rasterization

`content_layer_client_impl.cc` `Update()` diagnostics:
```
[AGENTMUX-TRANS-CLC] layer bg=(0,0,0,0)        isOpaque=0 rect_known_opaque=0 final_contents_opaque=0 bounds=1200x800
[AGENTMUX-TRANS-CLC] layer bg=(0.08,0.08,0.08,0.725) isOpaque=0 rect_known_opaque=0 final_contents_opaque=0 bounds=600x595
[AGENTMUX-TRANS-CLC] layer bg=(0,0,0,0.5)      isOpaque=0 rect_known_opaque=0 final_contents_opaque=0 bounds=600x149
[AGENTMUX-TRANS-CLC] layer bg=(0.08,0.08,0.08,0.725) isOpaque=0 rect_known_opaque=0 final_contents_opaque=0 bounds=600x743
```

No layer is marked `contents_opaque=true`. Tiles should rasterize as RGBA, preserving alpha. ✓

### Pixel sample (post all patches, software compositing)

```
( 50, 100) body/block gap         -> rgb(14, 77, 14) g_bleed=+63 ✓ GREEN BLEED
(200, 150) pane interior          -> rgb(62, 62, 62) g_bleed=+0  ✗ no green tint
(300, 200) pane interior 2        -> rgb(62, 62, 62) g_bleed=+0  ✗ no green tint
```

The body+block-only gap region DOES bleed wallpaper through. The body+block+agent-view stack region does NOT. Where does the opacity come from?

### Empirical verification

CSS-injection test (via Chrome DevTools Protocol Runtime.evaluate):
- Set `body { background: rgba(255, 0, 0, 0.5) !important }` and all other backgrounds transparent
- Expected if transparent: `rgb(127, 127, 0)` (red 0.5 over green wallpaper)
- Expected if opaque-white-backed: `rgb(255, 127, 127)` (red 0.5 over white)
- Measured at pane interior: `rgb(253, 125, 125)` — matches **opaque white** within 2 channel units

CSS-strip-everything test:
- All backgrounds set to `transparent !important`
- Pane interior: `rgb(250, 250, 250)` — pane area is filled with WHITE pixels even when no CSS background exists

**Conclusion:** something in Chromium's rasterization pipeline is producing opaque white pixels at pane-interior regions, regardless of CSS. The browser-side and renderer-side LayerTreeHost-level transparency is fully in place; the opacity comes from a per-layer/per-tile rasterization path.

---

## Hypotheses tried and discarded

1. **LCD text optimization forces `contents_opaque=true`** — added `--disable-lcd-text`. No visual change. CLC log confirms `final_contents_opaque=0` for all layers.

2. **`!= kTransparent` vs `isOpaque()` in `contents_opaque` check** — patched `content_layer_client_impl.cc`. The check now correctly only marks layers opaque when their bg is truly alpha=1. CLC log confirms all layers `contents_opaque=0`. No visual change.

3. **LayerTreeHost `background_color` stuck at opaque** — patched `web_view_impl.cc` `UpdateBaseBackgroundColor`. Now correctly propagates alpha < 1 to `cc::LayerTreeHost`. LTI log confirms `commit.bg=(0.133, 0.133, 0.133, 0.45098)`. No visual change at pane interiors.

4. **Renderer's screen-fill quad fills opaque** — confirmed via my CC patch that `will_fill_screen=0` (no fill quad appended). The opacity comes from somewhere else.

---

## Remaining hypothesis (UNVERIFIED — needs investigation)

`cc/raster/raster_source.cc` `PlaybackToCanvas`:
```cpp
if (!requires_clear_) {
    ClearForOpaqueRaster(...);  // clears with raster_source.background_color_, kSrc blend
} else if (!is_partial_raster) {
    raster_canvas->clear(SK_ColorTRANSPARENT);
}
```

And `cc/layers/picture_layer.cc:122`:
```cpp
recording_source.SetRequiresClear(!contents_opaque() && !client_->FillsBoundsCompletely());
```

If `client_->FillsBoundsCompletely()` returns true for the agent-view's PictureLayer, `requires_clear_` becomes FALSE (even with `contents_opaque=false`), and `ClearForOpaqueRaster` is called. The clear uses `recording_source.background_color_` set via `SetBackgroundColor(SafeOpaqueBackgroundColor())`.

`Layer::SafeOpaqueBackgroundColor()`:
- `contents_opaque=false`, `background_color().isOpaque()=false` (alpha 0.45)
- returns `background_color()` = `rgba(34, 34, 34, 0.45)`

This would clear the tile to alpha 0.45 — not opaque white. So this alone doesn't explain the observed opaque white either.

**Next step (for future investigation):**
1. Add LOG inside `RasterSource::PlaybackToCanvas` printing `requires_clear_`, `background_color_`, `is_partial_raster`, layer bounds
2. Also LOG inside `PictureLayer::Update` printing `contents_opaque()`, `client_->FillsBoundsCompletely()`, the resulting `requires_clear` value
3. Determine which path the agent-view's layer takes
4. If `requires_clear=false` and `background_color_` is opaque (which would be a bug), patch `SafeOpaqueBackgroundColor` or `PictureLayer::Update` accordingly

---

## Workaround / Acceptable state

The current state achieves PARTIAL transparency:
- Window borders, tab bar gaps, body-only regions bleed wallpaper through
- Multi-layer interior regions remain opaque

This is enough that the user can see the desktop wallpaper around the AgentMux window's edges and through gaps, but the pane content backgrounds are solid. Probably not the "full glassmorphic UI" effect intended, but it IS transparent in some areas.

If full transparency is needed:
- Continue investigation per "Remaining hypothesis" above
- OR avoid the issue by setting `window:transparent=false` and using a near-opaque `window:bgcolor` of `rgb(34, 34, 34)` — works without the cascade

---

## File map

- CEF fork patches: `~/cef-build/chromium_git/cef/libcef/browser/views/{window_impl.cc,window_view.cc,browser_view_impl.cc}`
- Chromium patches (NOT yet in CEF patch system): `~/cef-build/chromium_git/chromium/src/third_party/blink/renderer/core/exported/web_view_impl.cc`, `~/cef-build/chromium_git/chromium/src/third_party/blink/renderer/platform/graphics/compositing/content_layer_client_impl.cc`
- AgentMux: `frontend/app/app.tsx`, `agentmux-cef/src/app.rs`
- Settings: `~/.agentmux/versions/settings.json` has `window:transparent=true, window:opacity=0.45`
- Test artifacts: `/tmp/transparency-test/v792-FINAL-CLEAN.png` (latest)

## Build state at session end

- libcef.so HEAD-of-patches: `~/cef-build/chromium_git/chromium/src/out/Release_GN_x64/libcef.so` mtime 11:36, contains `web_view_impl.cc` (UpdateBaseBackgroundColor patch) + `content_layer_client_impl.cc` (isOpaque() patch) + CEF source patches
- AppImage extracted: `~/.local/share/agentmux/extracted/0.33.792/usr/bin/{agentmux,libcef.so}` updated 11:40
- Original AppImage on Desktop: `~/Desktop/AgentMux_0.33.792_amd64.AppImage` — built before today's chromium patches, would need a fresh `task package` if redistributing

---

## Session 3 addendum (2026-05-11 evening, 17:00–23:20 PDT)

Continued the investigation that ended session 2. New findings:

### What we tried and confirmed by pixel sampling

Diagnostic LOGs were added at every layer of the rasterization pipeline to identify the source of the opaque rgb(250, 250, 250) tile content:

1. **`SoftwareRenderer::DrawTileQuad` tile-content sampling** — confirmed tile bitmap center pixels are `0xfffafafa` (opaque rgb(250, 250, 250)) even after all CSS bgs are stripped via CDP injection. Source is below the CSS layer.

2. **`SoftwareRenderer::DrawSolidColorQuad` quad inspection** — found two opaque sources before the SI clamp patch:
   - Full-viewport `(0.133, 0.133, 0.133, 1)` (= body color, alpha=1) — a SolidColorLayer
   - 254×254 tile-grid `(0.980, 0.980, 0.980, 1)` quads (= Material Grey 50) — PictureLayer tiles detected as solid color

3. **`ViewPainter::PaintRootGroup` diagnostic** — captured the actual `root_element_background_color` painted into tiles. Early frames showed `#222222` opaque (i.e., body's color promoted to alpha=1) BEFORE AppSettingsUpdater ran. Later frames showed `rgba(34, 34, 34, 0.45)` correctly. The early-opaque paints get baked into tiles and the tile cache reuses them.

### Patches tried that DID NOT survive

These were brute-force / too-aggressive and were reverted at the user's "we want robust, not a hack" direction:

- `software_renderer.cc DrawTileQuad` — skip tile quads whose center pixel is opaque rgb >= 240 each. **DID produce visible transparency** (pane interior rgb(30, 35, 30) with green tint over wallpaper) but rejected as a hack.
- `pending_layer.cc::UsesSolidColorLayer` — globally reject non-opaque solid colors → forced PictureLayer path. Broke agentmux's UI rendering.
- `solid_color_layer_impl.cc::AppendQuads` — clamp quad alpha to active_tree.background_color alpha. Broke rendering.
- `web_frame_widget_impl.cc::SetBackgroundColor` — "sticky transparency" latch: once host bg is non-opaque, reject opaque updates. Broke rendering.
- `tile_manager.cc` — reject solid-color cache for near-white analyzed colors. Insufficient on its own.
- `raster_source.cc::ClearForOpaqueRaster` — always clear with kTransparent. Insufficient.
- `picture_layer_impl.cc` missing-tile fallback — kTransparent instead of safe_opaque. Insufficient.
- `view_painter.cc::PaintRootGroup` — skip opaque combined when base is transparent. Caused agentmux UI to not render at all (root view's bg fill is required).
- `aura/window.cc::OnFirstSurfaceActivation`, `delegated_frame_host_client_aura.cc::GetGutterColor`, `render_widget_host_view_aura.cc::CreateAuraWindow` — change SK_ColorWHITE defaults to TRANSPARENT. Each one is a real opaque-white source but reverting individually did not eliminate the pane-interior opacity.

### Final patch set (verified safe — agentmux UI renders fully)

**CEF source patches (`a5af/cef` `agentmux/7680-drag-rightclick-and-transparency` HEAD `3e041ad2f`):**
- `window_impl.cc::CreateWidget` — deferred `SetBackgroundColor(SK_ColorTRANSPARENT)` for top-level windows after `widget_` is assigned
- `window_view.cc` — mirror translucent branch for non-modal top-level windows
- `browser_view_impl.cc::TransparencyApplyOnRenderReady` — WebContentsObserver that calls RWHView::SetBackgroundColor + direct SetBackgroundOpaque(false) via owner_delegate

**Chromium patch (in local tree only — needs porting to CEF's `patch/patches/` system):**
- `third_party/blink/renderer/core/exported/web_view_impl.cc::UpdateBaseBackgroundColor` — re-push BackgroundColor() to FrameWidget so cc::LayerTreeHost.background_color gets the alpha-aware value

**AgentMux (`agentu/cef-transparency` HEAD `5d7ee44b`):**
- `frontend/app/app.tsx::AppSettingsUpdater` — `--window-opacity` set on `documentElement` (`:root`), not body
- `agentmux-cef/src/app.rs` — `--disable-lcd-text` Chromium switch

### Unresolved

After all the safe patches, pane interiors still render opaque rgb(34, 34, 34) — same as the original baseline. Window borders/gaps continue to show wallpaper bleed-through (rgb(14, 77, 14) green tint). The rgb(250, 250, 250) opaque-tile source was identified empirically (visible in tile bitmap samples) but its blink display-list source was never traced. The only patches that produced visible interior transparency caused other rendering regressions.

### Next investigation steps (for future sessions)

1. Identify the blink paint op that writes opaque rgb(250, 250, 250) to tiles. Hypothesis: `kColorWindowBackground` or `kColorPrimaryBackground` via Material Design ColorProvider for some implicit chrome element (scrollbar track? rootView default?).
2. Patch CEF's color provider to return transparent for `kColorWindowBackground` family when `is_transparent=true` is set.
3. Test on a system with working GPU (this machine has VAAPI/WebGL blocklisted, forcing software compositing — may behave differently with GL/Vulkan).

### Test artifacts

- `/tmp/transparency-test/v792-SAFE-FINAL.png` — final safe state (pane opaque, gaps transparent)
- `/tmp/transparency-test/v792-SKIP-WHITE-TILES.png` — brute-force tile-skip state (pane partial-transparent rgb(30, 35, 30))
