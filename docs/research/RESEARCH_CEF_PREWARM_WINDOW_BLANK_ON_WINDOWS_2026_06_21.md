# Research: pre-warmed CEF window renders blank on Windows (promotion blank-surface)

**Date:** 2026-06-21
**Method:** multi-source web research with adversarial (3-vote) claim verification — 20 sources, 80 claims extracted, 25 verified (**14 confirmed, 11 refuted**), 102 research sub-agents.
**Context:** AgentMux's window pool (`agentmux-cef/src/commands/window_pool.rs`) pre-spawns CEF top-level windows **visible but off-screen at `(-32000,-32000)`**, then on demand moves+resizes them on-screen and populates the DOM. On **Windows**, the promoted window has a correct DOM (CDP: full element tree, `visibilityState=visible`, on-screen at correct size) but the **visible surface is blank**. **macOS works.** Cold-created (on-screen at final size) windows render fine. `--disable-features=CalculateNativeWinOcclusion` (CEF #3638) did **not** fix it.

---

## 1. Root cause (confirmed): a Windows compositor **visibility-state desync**, not a GPU/swapchain allocation failure

The symptom (valid DOM, blank surface) is produced on Windows by the browser-compositor's *visibility state* getting stuck, so the compositor frame is never presented/kept. Three independently-documented mechanisms produce it; the first is the direct match for our pattern:

### 1a. The "already visible" defect — CEF #3638 (primary match)
The CEF maintainer's own diagnosis: the aura `Window` is marked **VISIBLE *before* the real Show/Restore**. Because it is already "visible," the later `Show` is **ignored in `Window::SetVisibleInternal` as "No change."** The genuine **hidden→visible transition** that the compositor / occlusion detector needs is therefore suppressed.

> *"The Window having visible status prior to Show (and prior to Restore) seems logically wrong. We want the Window to transition from hidden to visible when Restore is called."* — magreenblatt, cef#3638

**Why this is exactly our bug:** an **off-screen-but-VISIBLE** pre-warm window is already in the "visible" state. The promotion (SetWindowPos move + resize) does **not** generate a hidden→visible transition, so the compositor/occlusion state never re-syncs → blank. A genuinely *hidden* (never-shown) window transitions cleanly on first show. [confirmed 2-1; source: github.com/chromiumembedded/cef/issues/3638]

### 1b. Native window occlusion mis-classification
`NativeWindowOcclusionTrackerWin` can mark a window HIDDEN/OCCLUDED; when it does, *"rendering stops, and js is throttled,"* and *"if a window is falsely determined to be occluded, the content area will be white."* [confirmed 3-0; source: Chromium `docs/windows_native_window_occlusion_tracking.md`]

### 1c. Compositor-frame eviction (DelegatedFrameHost / FrameEvictor)
The browser-side visibility (`RenderWidgetHostImpl::is_hidden_` / `WasShown`) can desync so `DelegatedFrameHost::WasShown` → `FrameEvictor::SetVisible(true)` is skipped and the **compositor frame is evicted while the DOM stays valid and interactive** — the exact "DOM correct, surface blank" symptom. [confirmed 2-1; sources: electron#42378, electron#42638]

### Refuted along the way (do NOT rely on these)
- "Off-screen positioning (`-32000`) marks the window occluded" → **refuted 0-3.** The off-screen *position* is not the trigger; the *"already visible"* state is.
- "`--disable-features=CalculateNativeWinOcclusion` alone fixes it" → **refuted 0-3.**
- "`--disable-gpu` fixes it (proving a pure GPU/swapchain root cause)" → **refuted 0-3.**
- "Creating hidden via `SW_HIDE`→`SW_SHOW` itself triggers blank" → **refuted 0-3.**
- "CEF 139 blank is Windows-11-only" → **refuted 0-3.**

---

## 2. How Electron / VS Code avoid it

Electron's `BrowserWindow({ show: false })` is a **genuinely hidden, never-shown** window; the canonical lifecycle is `win.once('ready-to-show', () => win.show())`, which the docs state **"will have no visual flash."** This is a true hidden→visible transition — *not* an off-screen-but-visible window. [confirmed 3-0; source: electronjs.org BrowserWindow docs]

**Important nuance (verified, tempers the framing):** Electron's hidden-window path is **not flawless on Windows** either. Per maintainer-confirmed electron#32001: *"On Windows and Linux, paint events do not fire while the window is hidden… On macOS, it works as expected."* So on Windows a hidden window may **not pre-paint while hidden** — it paints correctly **after** `show()`. Related: electron#22670 (blank after hide/update/show on Windows, *"works fine on Mac OS"*) and electron#27353 (hidden BrowserView blank after show **until moved/resized/unfocused** — direct evidence that forcing a `WM_SIZE`/focus event re-syncs the compositor). [confirmed 3-0]

---

## 3. Why macOS works but Windows doesn't

macOS drives renderer/window visibility from **OS window occlusion** (Core Animation / IOSurface backing), so a hidden/pre-warmed window keeps a valid surface and paints. On Windows/Linux, visibility is tied to **explicit `show()`/`hide()`** and the **DirectComposition/Aura** compositor + native occlusion tracker, which can leave the surface stale/evicted/occluded. Same code, different result across OSes → the defect is localized to the Windows compositor/visibility stack, consistent with the IOSurface-vs-DirectComposition framing. [confirmed 3-0; sources: electron#32001, electron#22670, Electron docs]

---

## 4. Recommended fix direction (medium confidence — verify empirically)

Ranked, from the confirmed mechanisms:

- **(a) PREFERRED — pre-warm GENUINELY HIDDEN, not off-screen-visible.** Create pool windows **without `WS_VISIBLE` / `SW_HIDE`, never shown at `-32000`.** On promote: `SetWindowPos`(final rect, **real size delta**) → `ShowWindow(SW_SHOW)` → `UpdateWindow`, then force compositor resync (`CefBrowserHost::WasResized()`). This produces the real hidden→visible transition #3638 shows is required; it is the CEF analog of Electron `show:false` + `show()`.
- **(b) On promote, force the resync** even if (a) is partial: a real size change (→ `WM_SIZE`), `ShowWindow(SW_SHOW)`, `WasResized()`/`WasHidden(false)`.
- **(c) Last resort toggles** known to repaint: minimize→restore, `SW_HIDE`→`SW_SHOW`, or move/resize/unfocus (electron#27353).
- **(d) Mitigations, not standalone fixes:** `--disable-features=CalculateNativeWinOcclusion` + `--disable-backgrounding-occluded-windows` + `--disable-renderer-backgrounding`.

---

## 5. Caveats / open questions (carry into the implementation plan)

1. **No single source pins THE exact mechanism for our specific off-screen-visible CEF windowed-mode case.** Three documented Windows mechanisms each produce the symptom; confirm which one bites via an isolated repro (`about:gpu`, occlusion logging, toggling one lever at a time).
2. **CEF #3638 was fixed in M120+/2024.** If our CEF is recent, that exact defect may already be patched, shifting cause toward the occlusion-tracker / FrameEvictor paths or to off-screen-visible behavior not covered by #3638's fix.
3. **Hidden windows may not pre-paint on Windows** (electron#32001). So genuinely-hidden pre-warm likely keeps the renderer-spawn + page-load saving but does **not** keep a pre-painted surface; it paints on first show (no flash). If a *pre-painted* surface is required, a **layered/transparent on-screen** pre-warm (WS_EX_LAYERED alpha 0, on-screen so not occluded) is the alternative to evaluate.
4. CEF windowed mode uses a raw native HWND (child of the app HWND); Electron uses Views/aura + BrowserView abstractions, so its issues are a strong analog but not a 1:1 map. `Invalidate(PET_VIEW)` is **OSR-only** and won't help windowed mode.

---

## 6. Sources (primary unless noted)
- CEF #3638 — premature-visible / "No change" suppression: github.com/chromiumembedded/cef/issues/3638
- Chromium occlusion design doc (white content on false occlusion): github.com/chromium/chromium/blob/main/docs/windows_native_window_occlusion_tracking.md
- Electron #42378 + PR #42638 — FrameEvictor eviction / visibility desync
- Electron #32001 — paint events don't fire for hidden windows on Windows/Linux (macOS does)
- Electron #22670 — blank after hide/update/show on Windows, fine on macOS
- Electron #27353 (+ PR #29919) — hidden BrowserView blank until move/resize/unfocus
- Electron BrowserWindow docs — `show:false` + `ready-to-show` + `show()` (no visual flash)
- Electron #50250 — `setBackgroundThrottling(false)` early-return short-circuits visibility notifications (thematic)
- magpcss CEF forum threads (t=11672, t=20363) — corroborating, some claims refuted

*Verification stats: 5 search angles, 20 sources fetched, 80 claims extracted, 25 verified (14 confirmed / 11 refuted), 8 after synthesis, 102 agent calls.*
