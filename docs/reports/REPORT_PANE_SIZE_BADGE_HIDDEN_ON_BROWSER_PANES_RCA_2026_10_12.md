# RCA: the W×H resize badge never shows on browser panes

**Date:** 2026-10-12
**Status:** analysis — root cause identified from source; fix recommended (§6), not implemented.
**Author:** Agent1
**Trigger:** Repo owner, 2026-10-12: while resizing panes, a `W×H` label appears at the bottom-left of every pane except browser panes. Recent browser-pane performance work (which was itself a regression fix) must not regress.
**Code:** `main` @ `84e05ef4b` (rechecked after the 2026-10-13 RPC refactor; the badge code is unchanged, only moved).
**Related:** `SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md` (the badge), `analysis/ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md` (the perf work, #4420, with #4657 and #4690 for macOS), `analysis/ANALYSIS_BROWSER_PANE_AIRSPACE_ARCHITECTURE_2026_05_30.md`, `SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md`.

---

## 1. Summary

- **The badge is drawn, but under the page.** It is DOM. A browser pane's page is a native window placed over the pane, and a native window paints above the app's DOM whatever the CSS `z-index` (the "airspace" problem). The badge sits inside the area the native window covers, so it's hidden.
- **Nothing cuts a hole for it.** DOM that must show over a browser pane opts into the overlay clip registry (`usePaneOverlay` or `data-pane-overlay`). The badge doesn't. The other mechanism that clears a pane during a drag, the drag catcher, covers only pane, tab and document drags, not splitter drags.
- **It is not a regression, and the perf work didn't cause it.** The badge (#1057, 2026-05-26) has been unchanged since it shipped, and browser panes were already native windows then. It has never shown on a browser pane.
- **Recommended fix: draw the badge where the page isn't.** For a pane whose content is a native surface, anchor the badge to a DOM-only strip of the pane (the browser nav bar) instead of the bottom-left corner. It is CSS plus one capability flag, with no IPC and no host work, so the resize pipeline from #4420 is untouched (§6).
- **Not recommended:** registering the badge as a pane overlay. It looks like a one-liner, but it would add a clip update and a region change on every pane for every frame of a drag. On macOS and Linux it would hide whole panes for the length of the drag (§5).

## 2. Symptom

During a splitter drag, every pane shows a `W×H` badge at its bottom-left except browser panes. The badge appears on browser panes nowhere in the pane, not even partly.

## 3. Root cause

### 3.1 The badge is DOM at the pane's bottom-left, on every pane

- `frontend/app/block/blockframe.tsx:1439` mounts `<PaneSizeBadge>` on every pane, browser panes included, while `nodeModel.isSplitterDragging()` is true. That memo is true while the layout's pending action is a `ResizeNode`, meaning a splitter drag (`frontend/layout/lib/layoutModel.ts:489-492`).
- `frontend/app/block/pane-size-badge.scss:7-27` positions it `absolute` at `left: var(--space-2); bottom: var(--space-2)` (8 px in from the frame's bottom-left corner), `z-index: 30`.

### 3.2 A browser pane's page is a native window over the placeholder

- The page is not DOM. On Windows it is a `WS_CHILD` wrapper HWND holding a windowed CEF browser. On Linux and macOS it is a CEF Views `BrowserView` added as an overlay view, which on macOS is its own `NSWindow` (resize analysis §3.1).
- It is sized and placed to the `.browser-placeholder` element (`frontend/app/view/browser/browser-view.tsx:390`, measured by `use-pane-rect-sync.ts` and sent through `pane-rect-batcher.ts`).
- `.browser-placeholder` is `flex: 1` with a 2 px margin (`browser-view.scss:261-271`). It fills the pane from below the nav bar to 2 px above the frame's bottom edge and 2 px in from its sides.
- So the badge's box, 8 px in from the bottom-left corner, lies entirely inside the native window's rect.

### 3.3 Native windows paint above the DOM, whatever the z-index

This is the airspace problem `frontend/app/platform/pane-overlay.ts:4-14` describes. The app's own page is one surface; each browser pane is another surface layered above it by the OS compositor (DWM on Windows). CSS stacking can't reach across that boundary. The badge's `z-index: 30` orders it within the app's page only.

### 3.4 Nothing opens the page for the badge

- **The overlay clip registry** is how DOM shows over a browser pane. On Windows it cuts a `SetWindowRgn` hole in the pane's window (`crates/cef/src/browser_panes/clip.rs:303`). On macOS and Linux it hides the whole pane while an overlay intersects it (`pane-overlay.ts:174-196`). An element joins by calling `usePaneOverlay` or carrying `data-pane-overlay`. The badge does neither.
- **The drag catcher** (`browser-view.tsx:471`) covers the page with a snapshot and opens a hole for the length of an in-app drag. It is gated to `ELEMENT_DRAG_KINDS = ["tile", "window-tab", "pane-tab", "doc-tab"]` (`browser-view.tsx:37`). A splitter drag is none of these, so during it the page stays live and on top.

Both are deliberate. The catcher exists for drop targets, which splitter drags don't have, and keeping the page live while it is resized is the point of the perf work.

## 4. Not a regression

- `pane-size-badge.tsx` was added in #1057 (2026-05-26) and has not changed since.
- Browser panes were native windows before that (`SPEC_NATIVE_BROWSER_PANE_2026_04_17.md`), and the badge's spec even chose the bottom-left corner partly *for* browser panes (to avoid their scrollbar), without accounting for the page covering it.
- The perf work (#4420, #4657, #4690, #4718) changed how a pane's rect reaches the host. It touched neither the badge, the overlay registry, nor the catcher.

So the badge has been invisible on browser panes since it shipped, on every platform. The macOS and Linux paths cover the placeholder the same way; this conclusion comes from the code, and only Windows matches the owner's report.

## 5. Fix options, judged by what they cost during a drag

A splitter drag next to browser panes is exactly the case #4420 made fast: each frame, one batched rect update per window, with the host moving every pane in one deferred move (about 2 ms a pane). A fix should add nothing to that frame.

| Option | Works | Cost during a drag | Verdict |
|---|---|---|---|
| **A. Register the badge as a pane overlay** (`data-pane-overlay` or `usePaneOverlay`) | Windows: yes. macOS/Linux: by hiding the whole pane | The badge's rect changes every frame of the drag (it is bottom-anchored, and the pane is resizing), so the clip is re-sent every frame: one more IPC per window per frame, competing with the rect batch, and a `SetWindowRgn` plus repaint on every intersecting pane. The clip and the rect travel separately, so the hole and the pane can land in different frames and flicker. On macOS and Linux each intersecting pane is hidden for the whole drag, after waiting up to 300 ms for a freeze frame (`pane-overlay.ts:74`), so the page goes static, the opposite of the perf work's goal | **No** |
| **B. Draw the badge where the page isn't** (the nav bar strip, which is DOM) | Yes, on every platform | None: a different CSS position on a DOM element that already exists during the drag. No IPC, no host work | **Recommended** |
| **C. Freeze browser panes for splitter drags** (snapshot shown, native page hidden: the resize analysis's §5.5 "resize mode") | Yes | Saves GPU during a drag, but needs a snapshot ready at drag start, makes the page static while dragging, and changes how all browser panes behave in a drag | Only as part of the resize strategy, never for the badge alone |
| **D. Draw the badge natively in the host** | Yes | New native UI per pane, per platform | No: far too much for a debug-style readout |

Moving Windows to CEF Views overlays (the resize analysis's §5.2) would not fix this either: an overlay view still draws above the app's page.

## 6. Recommendation

**Option B.** Let a view say its content is a native surface, and put that view's badge in a DOM-only part of the pane.

- **Capability:** add a flag to `PaneTabCapabilities` (`frontend/app/block/pane-tab-registry.ts:28`), for example `nativeContentSurface?: boolean`, set for the browser view. It is read the same way `blockframe.tsx` already reads `statsBadgeSetting` and `hueBorder`, through `paneTabCapability(view, …)`.
- **Placement:** when the flag is set, add a modifier class to the badge so it anchors to the **top-left** of the content area, over the browser nav bar (`browser-view.scss:32-40`), instead of the bottom-left. The nav bar is DOM, so nothing covers it.
  - It covers the back and forward buttons only while a splitter is held, and it never takes input (`pointer-events: none`).
  - A top-right placement over the URL field also works; top-left is closest to the spec's intent.
- **Unchanged:** the badge's lifetime (mounted only during a splitter drag), its ResizeObserver on the frame, the numbers it shows (still the whole pane's frame), every other view type, and the whole rect pipeline: `use-pane-rect-sync.ts`, `pane-rect-batcher.ts`, `browser_panes/bounds.rs`.
- **Spec:** amend `SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md` §2.3 with the exception and the reason (a view whose content is a native surface can't host DOM over it).

**Verify** in an isolated `task dev` build on Windows, with a tab holding a terminal and one or more browser panes:
1. Drag the splitter between them. The badge shows on the browser pane at the nav bar, with numbers that track the drag, and still at bottom-left on the terminal.
2. Check the perf work didn't move. Re-run the fast-drag measurement from the resize analysis §2 and §7 with a pane splitter instead of the window edge: no new IPC request types during the drag, and rect request→reply in the same range as its §8 table.
3. Repeat on macOS and Linux if a build is at hand, since they share the frontend change.

## 7. Confidence and what wasn't done

- **Root cause: high.** Every link in §3 is in the code above, and the symptom matches it exactly: present on every pane type except the one whose content is a native window.
- **Not reproduced at runtime here.** I didn't run an isolated build to drag a splitter, so the "never shown on any platform" claim for macOS and Linux follows from the code, not observation.
- **No code changed.** This report is ahead of the fix, as the workspace rule for specs and reports asks.
