# CEF Wayland transparency — empirical test results

**Date:** 2026-05-11 00:01 PDT (updated 05:30 PDT — patch committed + rebuild in flight)
**Status:** Root cause identified, CEF source patch committed as `68e0dc668` ("views: complete the transparency cascade — propagate to WebContents") on the `agentmux/7680-drag-rightclick-and-transparency` branch. libcef.so rebuild in progress. AgentMux side flipped from `0xFF000000` → `0x00000000` at all three sites (CefSettings.background_color in main.rs, BrowserSettings.background_color in app.rs, `--background-color` CLI switch in app.rs).
**Related:** [`docs/research/cef-transparency-research-2026-05-10.md`](../research/cef-transparency-research-2026-05-10.md).

---

## What we did

End-to-end smoke test of the patched libcef.so transparency support, with the canonical CEF test apps (`cefsimple`, `cefclient`) and the test page (`transparent_views.html`) that the Chad Nelson patch shipped with. The goal: verify the source build works at the libcef level *before* wiring AgentMux.

Screenshots collected at `/tmp/transparency-test/cef{client,simple}-*.png`.

---

## What we verified is correct

### Source build

- Working tree: `~/cef-build/chromium_git/cef`, branch `agentmux/7680-drag-rightclick-and-transparency`, HEAD `d13654696`. All 6 fork commits applied (drag×4, transparency×1, right-click×1).
- Mirror tree under `chromium/src/cef/` — git HEAD is stale (`053d0a3cc`) but the rsync pipeline means the **working files are current**. Verified by diff'ing key transparency files between fork and mirror: identical.
- libcef.so on disk: 642MB, built May 2 07:38 from the current mirror.

### Patch presence in the binary

`strings libcef.so | grep -E 'is_transparent|SetBackgroundOpaque'` finds:

- `is_transparent` (the renamed parameter from `b921ffe18`'s context.cc change)
- `RenderViewHostImpl::SetBackgroundOpaque`
- `WebFrameWidgetImpl::SetBackgroundOpaque`
- `FrameWidgetProxy::SetBackgroundOpaque`
- `blink::mojom::FrameWidget::SetBackgroundOpaque_Sym` + the full Mojo IPC machinery

The renderer-side IPC chain (`SetBackgroundColor → SetBackgroundOpaque → cc::LayerTreeHost.has_transparent_background = true`) **is present** in libcef.so.

### Triggering path works at the CEF level

`cefsimple --no-sandbox --hide-frame --url=...transparent_views.html`:

- **Window is frameless** (no GNOME titlebar) → `IsFrameless()` is being honored at CefWindowDelegate level.
- The inner 600×400 rounded div renders correctly with `opacity:0.8` translucency (lighter grey than fully opaque). CSS-layer alpha works inside the page.
- `CefSettings::background_color` defaults to 0 (uninitialized zero-struct → alpha=0). Per the patched `CefContext::GetBackgroundColor`, this means `is_translucent = true` at Views Widget creation time, `params.opacity = kTranslucent`, `GetCefWindow()->SetBackgroundColor(SK_ColorTRANSPARENT)`.

In short: **every CEF-side step works.**

### cefclient adds an extra flag layer

`cefclient --use-views --hide-frame --transparent-painting-enabled --background-color=00000000`:

- Same outcome as cefsimple.
- `--transparent-painting-enabled` keeps `browser_background_color_` at 0, which means `MainContextImpl::PopulateSettings` doesn't override `settings->background_color` away from its zero-default.
- The "Using Chrome style; Views-hosted window; Windowed rendering" log line confirms the mode is correct.

---

## What we verified does NOT work — and why

**The Wayland surface is still opaque** in all four test screenshots, despite every CEF-side step succeeding. The CSS-layer alpha inside the window renders, but the window background outside the rounded div is opaque white.

### The actual blocker — Chromium-Wayland integration

The cefclient + cefsimple logs both show the same sequence of `Not implemented reached` errors followed by GPU compositing failure:

```
ERROR:ui/ozone/platform/wayland/host/wayland_seat.cc:91]
  Not implemented reached in WaylandSeat::OnName

ERROR:ui/ozone/platform/wayland/host/wayland_zwp_linux_dmabuf.cc:395]
  Not implemented reached in WaylandZwpLinuxDmabuf::OnTrancheFlags

ERROR:ui/ozone/platform/wayland/host/wayland_screen.cc:461]
  Not implemented reached in WaylandScreen::IsScreenSaverActive

ERROR:ui/ozone/platform/wayland/host/xdg_toplevel.cc:282]
  Not implemented reached in XdgToplevel::OnConfigureBounds

ERROR:ui/ozone/platform/wayland/host/xdg_toplevel.cc:289]
  Not implemented reached in XdgToplevel::OnWmCapabilities

ERROR:media/gpu/vaapi/vaapi_wrapper.cc:1640]
  vaInitialize failed: unknown libva error

ERROR:gpu/ipc/client/command_buffer_proxy_impl.cc:285]
  ContextResult::kTransientFailure: Failed to send GpuControl.CreateCommandBuffer

ERROR:services/viz/public/cpp/gpu/context_provider_command_buffer.cc:268]
  GpuChannelHost failed to create command buffer
```

The chain (best interpretation):

1. Mutter on this VM advertises Wayland protocol features (`zwp_linux_dmabuf_feedback_v1::tranche_flags`, `xdg_wm_base::wm_capabilities`, etc.) that Chromium 146.0.7680's Ozone-Wayland implementation doesn't handle.
2. Chromium logs the "Not implemented reached" diagnostics and falls back to a degraded path.
3. dmabuf import / GPU buffer sharing breaks → GPU context creation fails (`Failed to send GpuControl.CreateCommandBuffer`).
4. With GPU compositing disabled, Chromium uses the software compositor → submits bitmap-to-Wayland-surface.
5. The Views `kTranslucent` flag was *set* (we verified the patch ran), but the software-compositor's Wayland surface submission path does **not** propagate alpha; the wl_buffer is opaque despite the layer tree being marked transparent.

### Confirming experiments

Tried four configurations, all show the same opaque outcome:

| Config | Result |
|---|---|
| cefclient `--use-views --transparent-painting-enabled` | Framed + opaque (no `--hide-frame`) |
| cefclient `--use-views --hide-frame --transparent-painting-enabled --background-color=00000000` | **Frameless** + opaque around inner div |
| cefsimple `--hide-frame` | **Frameless** + opaque (same as cefclient with all flags) |
| cefsimple `--hide-frame --use-gl=angle --use-angle=swiftshader` | Frameless + opaque (forcing software GL didn't restore the path) |
| cefsimple `--hide-frame --ozone-platform=x11` | **Crashes** with `Check failed: ThreadCache::IsValid(tcache)` — XWayland path is broken in this Chromium build |

The patched libcef IS doing its job (frameless windows are correctly frameless). The path-to-Wayland-ARGB-surface is broken upstream of CEF — in Chromium's Ozone-Wayland layer when dmabuf negotiation fails.

---

## Implications for AgentMux

**This is NOT a CEF source-build problem.** No amount of rebuilding the current fork against the current Chromium 146.0.7680 base will produce transparency in this environment.

The current libcef.so on disk is correct, ships in the AgentMux AppImage (PR #743), and would produce a transparent window **on a system where Chromium-Ozone-Wayland's dmabuf path works**. We've not verified that. We have evidence it does NOT work here.

### Paths forward (ranked by effort)

#### A — Test on a different environment

Try the same `cefsimple --hide-frame` on:

1. **Bare metal Linux + Mutter** (non-VMware). Most likely to "just work" — dmabuf is the GPU-buffer-sharing protocol, hardware GPUs with current Mesa support all the features Chromium expects.
2. **Bare metal Linux + KWin / Hyprland**. KWin and wlroots-based compositors have well-tested transparency support and tend to advertise older/simpler dmabuf protocol versions.
3. **Different VM type** (QEMU+virtio-gpu instead of VMware SVGA3D). virtio-gpu's Mesa support is the most actively maintained.

If transparency works on (1), our current patched libcef.so is fine and we should ship it. The VM is a dev-only environment limitation.

#### B — Newer Chromium base

Rebase the a5af/cef fork onto a newer Chromium branch (147 / 148+). Newer Chromium releases include ongoing Ozone-Wayland fixes for emerging protocol versions in Mutter / KWin. This is a multi-day investment (the fork has 6 patches to re-rebase, and the OOM-safe build takes 3-6 hours).

The specific symptom (`OnTrancheFlags not implemented`) suggests Chromium needs newer dmabuf-v1 protocol handlers. Cross-reference with upstream Chromium commits to find where these were added; pick a base that includes them.

#### C — Force software path that emits ARGB

The current software-compositor fallback produces an opaque wl_surface even with `kTranslucent`. There may be a Chromium switch we haven't tried (`--enable-features=`, `--use-cmd-decoder=` flags) that forces the software path to emit ARGB. Worth a quick search of upstream Chromium issues for "wayland transparent software compositor".

#### D — Custom Chromium patch

Find the code path in Chromium's Ozone-Wayland software-compositor where `kTranslucent` should map to ARGB surface format. If the bug is local to that path, patch it ourselves. High risk — Chromium internals are large and Wayland-specific paths are sparsely documented.

### Recommendation

**A first.** Cheapest test: anyone with non-VMware Linux + Mutter can run

```bash
cd ~/cef-build/chromium_git/chromium/src/out/Release_GN_x64
LD_LIBRARY_PATH="$PWD" ./cefsimple --no-sandbox --hide-frame \
  --url="file://$(realpath ~/cef-build/chromium_git/cef/tests/cefclient/resources/transparent_views.html)"
```

and screenshot the window. If transparency appears: ship as-is, current libcef.so works on real hardware. If still opaque: B.

---

## Open questions

1. Does Electron exhibit the same `OnTrancheFlags not implemented` error on this exact VM? Electron has its own Wayland code path but shares much with Chromium upstream. If Electron also fails (we've been assuming it works because VSCode runs here), the answer may be "the VM stack is fundamentally broken for transparent Wayland surfaces, period."
2. Does VSCode's UI in fact use Wayland-side transparency, or just CSS-level alpha over an opaque window? Our assumption was VSCode = working transparency, but we never verified VSCode's window itself is transparent (vs. having an alpha-aware *theme*).
3. What's the upstream CEF roadmap on `agentmux/7680-...`'s status — has Chad Nelson's patch been merged anywhere we can compare against?

---

## Artifacts

Screenshots:
- `/tmp/transparency-test/cefclient-1.png` — `--use-views --transparent-painting-enabled` (no `--hide-frame`; framed)
- `/tmp/transparency-test/cefclient-2.png` — same as #1 (process_singleton attached to old instance)
- `/tmp/transparency-test/cefclient-3.png` — full flag set, frameless, opaque background
- `/tmp/transparency-test/cefsimple-1.png` — `cefsimple --hide-frame`, frameless, opaque background
- `/tmp/transparency-test/cefsimple-swiftshader.png` — adding `--use-gl=angle --use-angle=swiftshader`, same opaque outcome
- `/tmp/transparency-test/cefsimple-x11.png` — `--ozone-platform=x11` crashes before window appears

Logs:
- `/tmp/cefclient-transparency-test-3.log`
- `/tmp/cefsimple-test.log`
- `/tmp/cefsimple-swiftshader.log`
- `/tmp/cefsimple-x11.log`

---

## Next concrete step

Test on a real Linux box (or a non-VMware VM). If `cefsimple --hide-frame` shows a transparent window with the desktop visible behind the rounded grey div, the source build is verified end-to-end. We can then wire AgentMux confident that production will work, and document the VM-side limitation as a known dev-only quirk.

If it's still opaque on real hardware → escalate to path B (newer Chromium base) or path D (Chromium-side patch).

---

## UPDATE 2026-05-11 00:30 — actual root cause found via `WAYLAND_DEBUG=1`

The "not implemented" Wayland errors are red herrings. Confirmed by tracing every Wayland protocol call:

```bash
WAYLAND_DEBUG=1 ./cefsimple --no-sandbox --hide-frame \
    --url=file:///.../transparent_views.html 2>&1 | grep set_opaque_region
```

Output: **zero matches**. Chromium does NOT call `wl_surface_set_opaque_region` for this window. The two AgentMux patches at `wayland_window.cc:994` and `wayland_frame_manager.cc:438` (uncommitted in the Chromium tree, both gating on `IsOpaqueWindow()`) ARE in the libcef.so binary AND are firing — `opacity_` is correctly `kTranslucentWindow`, `IsOpaqueWindow()` returns false, all five `set_opaque_region` call sites bypass.

Also confirmed via the same WAYLAND_DEBUG log:

```
-> wl_shm_pool#23.create_buffer(new id wl_buffer#43, 0, 800, 600, 3200, 0)
                                                     ^
                                                     format=0 = WL_SHM_FORMAT_ARGB8888
-> wl_surface#24.attach(wl_buffer#43, 0, 0)
```

The wl_buffer attached to the main surface is **ARGB8888** — alpha channel is present. Wayland-side everything is correct.

**The pixels in the buffer are opaque because the renderer paints them opaque.** Specifically: `cc::LayerTreeHost::has_transparent_background_` stays at the default `false`, so Chromium's compositor clamps every fragment's alpha to 1.0 before submitting to the buffer.

To flip `has_transparent_background_`, something must call:

```cpp
content::RenderWidgetHostViewBase::SetBackgroundColor(SK_ColorTRANSPARENT);
```

That method (verified at `content/browser/renderer_host/render_widget_host_view_base.cc`) — when the new color is transparent and previous was opaque — calls:

```cpp
host()->owner_delegate()->SetBackgroundOpaque(false);
```

which routes to `RenderViewHostImpl::SetBackgroundOpaque(false)` (the IPC machinery confirmed via `strings libcef.so`), which proxies to `blink::WebFrameWidgetImpl::SetBackgroundOpaque(false)` in the renderer, which finally sets `has_transparent_background_ = true` in the cc LayerTreeHost.

**Chad Nelson's patch does NOT call this.** It calls (a) `CefBrowserViewImpl::SetBackgroundColor(transparent)` — Views-side only, and (b) `CefWindow::SetBackgroundColor(SK_ColorTRANSPARENT)` — also Views-side only. Both set the *Aura layer* background but never reach the RenderWidgetHostView. The renderer keeps stamping opaque pixels into the (otherwise-correctly-allocated) ARGB buffer.

This explains why the LD_PRELOAD "bitbang" worked when our source patch doesn't: the bitbang almost certainly was NOT just NULL-ing opaque_region (we proved that's irrelevant here). It probably patched libcef.so to either default `has_transparent_background_` to true OR to force-call `SetBackgroundOpaque(false)` somewhere. We need the equivalent source-level intervention.

---

## The missing CEF source patch

Add one source change to the a5af/cef fork at one of these call sites (any of them works — pick the cleanest):

### Option 1 — In `CefBrowserViewImpl::SetDefaults` (already touched by the existing patch)

After the existing `SetBackgroundColor(...)` Views-side call, also reach into the WebContents and propagate to the renderer view:

```cpp
void CefBrowserViewImpl::SetDefaults(const CefBrowserSettings& settings) {
  const SkColor bg = CefContext::Get()->GetBackgroundColor(&settings, STATE_ENABLED);
  SetBackgroundColor(bg);

  // AgentMux follow-up: also propagate to the renderer's RenderWidgetHostView
  // so cc::LayerTreeHost::has_transparent_background_ flips. The Views-side
  // SetBackgroundColor above only colors the Aura layer; without this second
  // call the compositor clamps fragment alpha to 1.0 regardless of how the
  // window/buffer is configured.
  if (SkColorGetA(bg) == SK_AlphaTRANSPARENT && browser_) {
    if (auto* view = browser_->GetWebContents()->GetRenderWidgetHostView()) {
      view->SetBackgroundColor(SK_ColorTRANSPARENT);
    }
  }
}
```

Caveat: `browser_` may not be available yet at `SetDefaults` time — it's called before the browser is created in some paths. Need to verify call ordering, or move the call to `OnBrowserAttached` / equivalent.

### Option 2 — In `AlloyBrowserHostImpl::OnRenderViewReady` (or Chrome runtime equivalent)

Most reliable point: after the renderer is ready, set the renderer view's color. Need to find the matching callback in both Alloy and Chrome runtime paths.

### Option 3 — `WebContents::SetPageBaseBackgroundColor(std::nullopt)`

The modern Chromium API explicitly lets a caller declare "no opaque base color" — null means "respect alpha". Easier to apply uniformly via `WebContentsImpl::SetPageBaseBackgroundColor`.

```cpp
browser_->GetWebContents()->SetPageBaseBackgroundColor(std::nullopt);
```

### Recommendation

Try Option 1 first (smallest change, adjacent to the existing patch). If browser_ isn't available at SetDefaults time, move to Option 3 inside the browser-attached callback.

### Build cost

After editing the CEF source, run the existing ninja-with-retry pipeline (memory `cef_build_in_progress.md`). Expect 5-30 minutes for an incremental rebuild (we're modifying a single CEF source file, no Chromium-side changes).

---

## Open question for the user

The "bitbang verified" result the user remembers — was it produced with a libcef.so that contained an explicit `RenderWidgetHostView::SetBackgroundColor(SK_ColorTRANSPARENT)` call somewhere (i.e., similar to the patch above)? Or was it the LD_PRELOAD shim that I just disproved? If the user remembers a libcef.so binary that worked WITHOUT our test-failing path, that binary's diff vs current is the spec.
