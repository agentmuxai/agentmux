# REPORT — Zoom in the Media viewer, and what else it should do

**Date:** 2026-10-09
**Status:** analysis — the zoom in §2 is built in the PR that adds this report; §3 to §5 are recommendations.
**Author:** Korp (narko)
**Trigger:** Owner request: "we want to introduce zoom into the Media viewer. in that case, simply scrolling (no need for [a modifier]) will zoom in. also, research best practices for any other features, write report to file".
**Method:** Read the Media pane (`frontend/app/view/media/`) on `main` at `f1acbe33d`. Compared how a dozen viewers and libraries handle zoom, pan, fit and live reload, from their docs and source (VS Code's image preview, Chrome's image document, Windows Photos, macOS Preview, Figma, Photopea, GIMP, Krita, ImageGlass, IrfanView, XnView, Google Photos, OpenSeadragon, PhotoSwipe, Panzoom, tev, HDRView). Sources in §7.

## 1. Summary

- **Plain-wheel zoom is right for a viewer.** Document tools (VS Code, GIMP, Figma) make the wheel scroll and need a modifier to zoom, but dedicated viewers (Windows Photos, ImageGlass, OpenSeadragon) zoom on the plain wheel [3][10][11]. The Media pane is a viewer, so the owner's choice matches them.
- **Built now (§2):** the wheel zooms about the cursor, a drag pans, double-click toggles fit and actual size, `+ - 0 1` and the arrow keys work, pixels show as squares when enlarged, and a newer render of the same size keeps the zoom.
- **The next most valuable additions (§4):** a background toggle for transparent images, ComfyUI's workflow and prompt from a PNG's metadata, copy and reveal-in-folder, and previous/next file in the folder.

## 2. What the zoom does (built)

| Input | Effect |
|---|---|
| Wheel | Zooms about the cursor, about 20% a notch; a trackpad's small deltas zoom smoothly. Line and page wheel units are converted. |
| Ctrl/Cmd+wheel | Unchanged: zooms the pane's UI, as it does in every pane. |
| Drag, when zoomed | Pans. The image can't be dragged out of view: centred on an axis where it fits, its edge stops at the pane's edge where it doesn't. |
| Double-click | From fit, to actual size (100%) at the cursor, or 2× for an image no bigger than the pane; from any zoom, back to fit. Matches Chrome's image document and PhotoSwipe [4][18]. |
| `+` / `-` | Zoom about the centre. |
| `0` / `1` | Fit / actual size, as in GIMP and Figma [6][8]. |
| Arrow keys, when zoomed | Pan a tenth of the pane. |
| "250% · Fit" button | Shown while zoomed in; shows the zoom as a percentage of the image's own size, and fits again. |

- **Range:** from fit (the minimum) to 32 image pixels per screen pixel, and never less than 4× the fitted size. GIMP and Figma go to 25600% [6][8]; 3200% is plenty to inspect a render.
- **Pixels:** past 2 screen pixels per image pixel the image draws with `image-rendering: pixelated` (VS Code switches at 3× [2]).
- **Live renders:** when a newer file lands and replaces the image, the zoom and pan stay if it has the same dimensions, as XnView's and IrfanView's "lock zoom" do [21][22]; a different size goes back to fit. Comparing one render to the next at the same spot is the point of watching the folder.
- **How:** one `translate() scale()` transform on the fitted image, so zooming never re-runs layout [5]. The pointer maths is in the view's CSS pixels, so it stays right when the pane itself is zoomed. Code: `media-zoom.ts` (the maths, unit-tested) and `media-view.tsx`.

## 3. Decisions for the owner

1. **Trackpad pinch.** Chromium sends a pinch as a wheel event with `ctrlKey` set, the same as Ctrl+wheel, and no reliable test tells them apart [12][15]. Today Ctrl+wheel zooms the pane's UI everywhere, so a pinch over an image zooms the pane, not the image. VS Code instead takes every Ctrl+wheel over the image for the image [2].
   - Recommended: keep it consistent with the rest of AgentMux for now, and revisit if trackpad users ask.
2. **A setting for the wheel.** With plain-wheel zoom, a two-finger trackpad scroll also zooms, which browsers can't tell from a wheel [15]. Windows Photos and ImageGlass make it a setting [10][11].
   - Recommended: add "Wheel in the Media pane: Zoom / Scroll" when someone asks; the default stays Zoom.
3. **Zooming out below fit.** Today fit is the minimum. Most viewers allow a little below it (OpenSeadragon 0.9× [3]), which helps little for single images.
   - Recommended: leave as is.

## 4. Other features, by value for effort

| # | Feature | Why here | Effort |
|---|---|---|---|
| 1 | **Background toggle: dark, light, checkerboard** | Renders and icons often have transparency, which is invisible on a dark pane [20]. | Low |
| 2 | **PNG metadata panel: ComfyUI workflow and prompt** | ComfyUI writes its `prompt` (API graph) and `workflow` (UI graph) as JSON in the PNG's `tEXt` chunks [29]. "Copy workflow" turns any render back into the graph that made it. | Medium |
| 3 | **Copy image; reveal in folder; open with the system viewer** | The everyday ways out of a viewer. The clipboard takes only `image/png`, so other formats go through a canvas first [30]. | Low |
| 4 | **Previous / next file in the folder (← →)** | The pane already watches the folder. At fit the arrows step through files; zoomed in they pan, as in Windows Photos and Google Photos [16][17]. | Low to medium |
| 5 | **Size and zoom readout** | W×H and zoom % in a small overlay that fades after use (VS Code puts it in the status bar [1]). | Low |
| 6 | **Compare two renders** | Start with an A/B flip key between a pinned reference and the newest render, as tev does [24]. Then swipe and onion skin, as GitHub's image diff does [31]. | Medium |
| 7 | **Video: loop short clips, speed, frame step** | Loop on for short clips; `<` `>` speed in 0.25× steps; `,` `.` step a frame while paused (YouTube's keys [32]). `requestVideoFrameCallback` says which frame is showing; seeking alone isn't frame-accurate [33]. Video zoom reuses this transform. | Medium |
| 8 | **Space+drag and middle-click pan** | Muscle memory from Figma and Photoshop [28]. | Low |
| 9 | **Fullscreen** | Look at a render without the rest of the app. | Low |
| 10 | **Pixel grid and colour picker** | Krita shows a grid past a zoom threshold [27]; ImageGlass picks RGBA/HEX [20]. For people checking generated textures and UI screenshots. | Medium |
| 11 | **Audio waveform** | wavesurfer.js, with regions to loop a section [34]. | Medium |
| 12 | **Rotate** | Rarely useful for renders and screenshots. | Low |

## 5. Pitfalls to keep avoiding

- **`will-change: transform` left on blurs a zoomed image**: Chromium rasters the layer once and scales that bitmap [35]. Not used here.
- **Wheel listeners**: one on the window or document is passive by default, so `preventDefault` does nothing; this one is on the pane's element with `passive: false` [36].
- **CSS zoom on an ancestor**: `getBoundingClientRect` is in zoomed pixels and `offsetWidth` isn't [37]. The view converts with their ratio, so the cursor stays on the same image point when the pane is zoomed.
- **Very large images**: decoded memory is about width × height × 4 bytes, and around 16384 px a side is a practical limit [25]. Past that, a downscaled preview (`createImageBitmap` with `resizeWidth`) or tiling, as OpenSeadragon does, is the answer. Nothing in the pane handles this yet.
- **Swapping a live render without a flash**: decode the new file off-screen (`img.decode()`) before swapping it in [25]. The pane fades the new image in today; decode-then-swap would remove the fade.
- **Accessibility**:
  - keyboard zoom and pan are in (WCAG 2.1.1 [38]);
  - drag-to-pan still needs a single-pointer alternative, such as click-to-centre or a minimap (WCAG 2.5.7 [39]);
  - a zoom change could be announced in a `role="status"` region (WCAG 4.1.3 [40]);
  - nothing here animates, so reduced motion is respected [41].

## 6. Not covered

- Image editing of any kind (crop, annotate). That belongs to another tool.
- Formats beyond what Chromium decodes (PSD, EXR, RAW). tev and HDRView exist for EXR [24].

## 7. Sources

1. https://github.com/microsoft/vscode/issues/42130 ; https://github.com/microsoft/vscode/issues/84406
2. https://raw.githubusercontent.com/microsoft/vscode/main/extensions/media-preview/media/imagePreview.js
3. https://openseadragon.github.io/examples/ui-zoom-and-pan/
4. https://photoswipe.com/options/ ; https://photoswipe.com/click-and-tap-actions/
5. https://github.com/timmywil/panzoom
6. https://help.figma.com/hc/en-us/articles/360041065034-Adjust-your-zoom-and-view-options
7. https://support.apple.com/guide/preview/keyboard-shortcuts-cpprvw0003/mac
8. https://docs.gimp.org/2.10/en/gimp-view-zoom.html ; https://docs.gimp.org/3.0/en/key-reference-view.html
9. https://www.photopea.com/learn/navigation
10. https://www.elevenforum.com/t/change-mouse-wheel-behavior-for-photos-app-in-windows-11.18682/
11. https://github.com/d2phap/ImageGlass/issues/2465
12. https://danburzo.ro/dom-gestures/ ; https://kenneth.io/post/detecting-multi-touch-trackpad-gestures-in-javascript
13. https://github.com/Comfy-Org/ComfyUI_frontend/issues/17729
14. https://developer.mozilla.org/en-US/docs/Web/API/WheelEvent/deltaMode
15. https://www.mappedin.com/resources/blog/why-panning-and-zooming-in-a-web-app-cant-be-perfect/
16. https://winaero.com/the-list-of-keyboard-shortcuts-for-photos-app-in-windows-10/
17. https://defkey.com/google-photos-shortcuts
18. https://chromium.googlesource.com/chromium/src/+/eab1ec606de5921ae4e78ce5ce690076e0a37a71/third_party/WebKit/Source/core/html/ImageDocument.cpp
19. https://github.com/microsoft/vscode/pull/340051
20. https://imageglass.org/news/new-with-the-imageglass-5-0-62 ; https://imageglass.org/news/announcing-imageglass-5-5-64
21. https://newsgroup.xnview.com/viewtopic.php?t=46193&p=193236
22. https://www.irfanview.net/help/guida/hlp_lock_zoom.htm
23. https://openseadragon.github.io/docs/OpenSeadragon.html
24. https://github.com/Tom94/tev ; https://github.com/wkjarosz/hdrview
25. https://developer.mozilla.org/en-US/docs/Web/API/HTMLImageElement/decode ; https://developer.mozilla.org/en-US/docs/Web/API/Window/createImageBitmap
26. https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/image-rendering
27. https://docs.krita.org/en/reference_manual/preferences/display_settings.html
28. https://help.figma.com/hc/en-us/articles/1500004414582-Pan-and-zoom-in-FigJam
29. https://www.numonic.ai/blog/comfyui-png-metadata-chunks-workflow-parameters
30. https://web.dev/async-clipboard/ ; https://developer.mozilla.org/en-US/docs/Web/API/ClipboardItem/ClipboardItem
31. https://docs.github.com/en/repositories/working-with-files/using-files/working-with-non-code-files ; https://github.blog/news-insights/behold-image-view-modes/
32. https://vidiq.com/blog/post/youtube-frame-by-frame/
33. https://web.dev/articles/requestvideoframecallback-rvfc
34. https://wavesurfer.xyz/plugins/regions
35. https://developer.chrome.com/blog/re-rastering-composite
36. https://developer.chrome.com/blog/scrolling-intervention-2/
37. https://github.com/microsoft/vscode/issues/233692
38. https://www.w3.org/WAI/WCAG22/Understanding/keyboard.html
39. https://www.w3.org/WAI/WCAG22/Understanding/dragging-movements.html
40. https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html
41. https://w3.org/WAI/WCAG21/Understanding/animation-from-interactions
