# AgentMux Zoom System Architecture

## 1. Current State

AgentMux has two independent zoom systems:

**Per-pane zoom** — scales terminal font size via block metadata (`term:zoom`). Each terminal pane has its own zoom level stored server-side. This is the primary zoom mechanism users interact with.

**Chrome zoom** — scales the window header (title bar) and status bar via the `--zoomfactor` CSS custom property. This is a cosmetic feature for users who want larger/smaller chrome UI.

The old architecture used Tauri's `window.set_zoom()` for global webview zoom. `--zoomfactor` and `--zoomfactor-inv` were *compensators*: when the window zoomed everything 1.5x, specific elements used `calc(Npx * var(--zoomfactor-inv))` to counteract the scaling and stay at their original size. That system has been fully removed. `window.set_zoom()` is now pinned to `1.0` at startup (`wave.ts:384-386`), and `--zoomfactor-inv` has been deleted.

### What exists now

| Component | File | Mechanism |
|-----------|------|-----------|
| Zoom store | `store/zoom.ts` | `chromeZoomAtom`, per-pane `zoomBlockIn/Out`, `zoomIn/Out/Reset` |
| CSS var declaration | `tailwindsetup.css:73-75` | `:root { --zoomfactor: 1; }` |
| CSS var application | `zoom.ts:119-121` | `applyChromeZoomCSS()` sets `--zoomfactor` on `documentElement` |
| Init | `wave.ts:381-391` | Pins Tauri zoom to 1.0, calls `initChromeZoom()` |
| Keyboard bindings | `keymodel.ts:685-719` | `Ctrl/Cmd +/-/0` → `zoomIn/Out/Reset` (per-pane only) |
| Scroll handler | `app.tsx:204-238` | `Ctrl+wheel` dispatches to chrome or pane zoom based on target |
| Terminal font | `termViewModel.ts:250-290` | `termZoomAtom` → `fontSizeAtom` (base * zoom) |
| Terminal update | `term.tsx:176-182` | In-place `terminal.options.fontSize` update, no reconstruction |
| Zoom indicator | `zoomindicator.tsx` | Transient overlay showing current zoom % |
| Block target | `blockframe.tsx:677` | `data-blockid` attribute on pane container |

---

## 2. The Problem

`--zoomfactor` was designed as a compensator. Now that global window zoom is gone, it's been repurposed for chrome scaling. But the implementation is incomplete:

**What responds to `--zoomfactor`:**
- `.window-header` — `height: calc(33px * var(--zoomfactor))`, `font-size: calc(13px * var(--zoomfactor))`
- `.status-bar` — `height: calc(22px * var(--zoomfactor))`, `font-size: calc(11px * var(--zoomfactor))`
- `.status-version` — `font-size: calc(10px * var(--zoomfactor))`
- `.status-bar-popover` — `font-size: calc(11px * var(--zoomfactor))`

**What does NOT respond:**
- Tab names — hardcoded `font-size: 11px` (`tab.scss:69`)
- Tab dimensions — hardcoded `max-width: 200px`, `min-width: 60px` (`tab.scss:7-8`)
- Tab close button — hardcoded `width: 20px`, `height: 20px` (`tab.scss:89-90`)
- Tab bar add button — hardcoded `width: 28px`, `height: 27px`, icon `font-size: 11px` (`tabbar.scss:55-56,69`)
- Tab separator — hardcoded `height: 14px` (`tab.scss:24`)
- Pinned tab spacer — hardcoded `height: 18px` (`tabbar.scss:43`)
- Action widgets — hardcoded `gap: 4px` (`action-widgets.scss:8`)
- Status bar items — hardcoded `gap: 3px`, `padding: 0 5px` (`StatusBar.scss:43-44`)
- Status bar icons — hardcoded `font-size: 9px` (`StatusBar.scss:57`)

**The result:** At `--zoomfactor: 1.5`, the header and status bar containers grow (height, base font-size), but tabs, icons, buttons, and spacing inside them stay at their original pixel sizes. The chrome looks broken — oversized containers with undersized children.

---

## 3. Architecture Options

### Option A: CSS `zoom` property on containers

```scss
.window-header {
    zoom: var(--zoomfactor);
    // Remove all calc() wrappers from height/font-size
    height: 33px;
    font-size: 13px;
}
.status-bar {
    zoom: var(--zoomfactor);
    height: 22px;
    font-size: 11px;
}
```

**Pros:**
- One property per container scales everything uniformly — fonts, icons, padding, gaps, borders, child elements with hardcoded px values all scale together
- Zero changes needed in child SCSS files (tab.scss, tabbar.scss, action-widgets.scss, etc.)
- Simple to implement and maintain; new UI elements "just work"
- CSS `zoom` is now in the CSS spec (Baseline 2024), supported in all modern browsers/webviews

**Cons:**
- The container takes up more actual layout space (the scaled size), which affects flex layout
- Sub-pixel rendering: at non-integer zoom levels (1.1x, 1.3x), text and borders may render slightly blurry
- `zoom` affects `getBoundingClientRect()` and mouse coordinates — drag-and-drop on tabs may need coordinate adjustment
- Nested `zoom` values multiply; must not set `zoom` on children

**Effort:** Low. ~10 lines of CSS changes total.

### Option B: `transform: scale()` on containers

```scss
.window-header {
    transform: scale(var(--zoomfactor));
    transform-origin: top left;
    // Manually set width/height to account for scaling
}
```

**Pros:**
- Uniform scaling like `zoom`
- Does not affect layout flow (the element occupies its original, unscaled space)

**Cons:**
- The element visually overflows its layout box — requires manual size compensation on a wrapper
- More complex than `zoom` because you need a wrapper element to reserve the correct space
- Click targets and coordinate systems are transformed — more complex event handling
- Text rendering quality may be worse than CSS `zoom`

**Effort:** Medium. Requires wrapper elements and manual size bookkeeping.

### Option C: Propagate `calc(Npx * var(--zoomfactor))` to every value

Apply `calc(Npx * var(--zoomfactor))` to every `font-size`, `width`, `height`, `padding`, `gap`, `margin`, `border-radius`, and icon size in every child SCSS file.

**Pros:**
- Precise control over exactly what scales
- No coordinate system surprises
- Layout sizes are always correct

**Cons:**
- Tedious: requires touching every hardcoded px value across ~6 SCSS files and dozens of properties
- Error-prone: miss one property and that element looks wrong at non-1.0 zoom
- Maintenance burden: every new UI element in the chrome must use the pattern
- Some values (box-shadow, border-width) look odd when scaled

**Effort:** High. 50+ individual property changes, ongoing maintenance cost.

### Option D: Font-size cascade with em/rem units

Set `font-size` on chrome root containers, convert all children to use `em` units.

**Pros:**
- Clean CSS architecture
- Text scales naturally

**Cons:**
- Only text scales. Icons (if using px-sized font icons), padding, gaps, heights, widths stay fixed
- Converting an existing px-based codebase to em is a large refactor
- `em` compounds in nested elements, making values harder to reason about
- Still need explicit handling for non-text dimensions

**Effort:** High. Full SCSS refactor, and still incomplete (non-text elements don't scale).

---

## 4. Recommendation

**Option A (CSS `zoom` property)** is the clear winner.

**Rationale:**
1. It solves the core problem — uniform scaling of all chrome elements — with minimal code changes
2. Zero maintenance burden on child components; new elements scale automatically
3. The tab drag-and-drop coordinate concern is the only real risk, and it's easy to test and fix (divide by zoom factor in the drag handler)
4. CSS `zoom` is now a web standard (not just a webkit extension) and Tauri's WebView2/WebKit both support it
5. The existing `calc()` wrappers on `height` and `font-size` in `.window-header` and `.status-bar` can be simplified back to plain px values

**Migration steps:**
1. Add `zoom: var(--zoomfactor)` to `.window-header` and `.status-bar`
2. Remove all `calc(Npx * var(--zoomfactor))` expressions from those files and their children — revert to plain `Npx`
3. Test tab drag-and-drop at zoom levels other than 1.0; if coordinates are off, apply zoom correction in the drag handler
4. Remove `--zoomfactor-inv` references if any remain (believed already removed)

---

## 5. Per-Pane Zoom Architecture

The per-pane zoom path is clean and well-architected. No changes needed.

### Data path

```
block metadata "term:zoom" (server-side, persisted)
    ↓
termZoomAtom (derived atom, reads blockAtom → meta["term:zoom"])
    ↓
fontSizeAtom (derived atom: baseFontSize * zoomFactor, clamped 4-64px)
    ↓
terminal.options.fontSize (in-place update, no terminal reconstruction)
    ↓
fitAddon.fit() (reflows terminal grid to new cell size)
```

### Key design decisions

- **Zoom is metadata, not local state.** Stored via `SetMetaCommand` RPC, so it persists across sessions and is visible to the backend (for `stty rows/cols` recalculation).
- **Null means default.** When zoom is exactly 1.0, the metadata key is set to `null` (deleted), keeping block metadata clean.
- **No terminal reconstruction.** Font size is updated via `terminal.options.fontSize` assignment (term.tsx:179), which avoids destroying/recreating WebGL contexts. The previous approach of including `termFontSize` in the construction useEffect's dependency array caused all terminals to re-render when any terminal zoomed.
- **Validation at every layer.** `termZoomAtom` clamps to 0.5-2.0. `fontSizeAtom` clamps the final result to 4-64px. `setBlockZoom` in zoom.ts also clamps and rounds.

---

## 6. Data Flow Diagrams

### Keyboard zoom (Ctrl+/-, Ctrl+0)

```
Ctrl+= / Ctrl+-
    ↓
keymodel.ts globalKeyMap handler
    ↓
zoomIn(store) / zoomOut(store) / zoomReset(store)     [zoom.ts]
    ↓
getFocusedBlockId()                                     [global.ts]
    ↓
getBlockZoom(blockId) → current term:zoom from blockAtom
    ↓
setBlockZoom(blockId, newFactor)
    ↓
RpcApi.SetMetaCommand → { "term:zoom": factor|null }   [server round-trip]
    ↓
blockAtom updates (WOS subscription)
    ↓
termZoomAtom recomputes
    ↓
fontSizeAtom recomputes (base * zoom)
    ↓
term.tsx useEffect[termFontSize]
    ↓
terminal.options.fontSize = termFontSize
fitAddon.fit()
```

### Scroll wheel over pane (Ctrl+wheel)

```
Ctrl+wheel event on document
    ↓
AppZoomHandler (app.tsx) wheel listener
    ↓
e.target.closest("[data-blockid]") → blockId           [blockframe.tsx:677]
    ↓
zoomBlockIn(blockId) / zoomBlockOut(blockId)            [zoom.ts]
    ↓
(same path as keyboard from setBlockZoom onward)
```

### Scroll wheel over chrome (Ctrl+wheel on header/statusbar)

```
Ctrl+wheel event on document
    ↓
AppZoomHandler (app.tsx) wheel listener
    ↓
e.target.closest(".window-header") or ".status-bar" → true
    ↓
chromeZoomIn() / chromeZoomOut()                        [zoom.ts]
    ↓
setChromeZoom(newFactor)
    ↓
globalStore.set(chromeZoomAtom, clamped)
    ↓
applyChromeZoomCSS(clamped)
    ↓
document.documentElement.style.setProperty("--zoomfactor", factor)
    ↓
CSS recalc: .window-header and .status-bar height/font-size update
(with Option A: zoom property scales all children uniformly)
```

---

## 7. Files Inventory

| File | Role |
|------|------|
| `frontend/app/store/zoom.ts` | Central zoom module. Per-pane functions (`zoomIn/Out/Reset`, `zoomBlockIn/Out`), chrome functions (`chromeZoomIn/Out/Reset`), zoom indicator, constants. |
| `frontend/app/app.tsx` | `AppZoomHandler` component. Ctrl+wheel listener that dispatches to pane or chrome zoom based on DOM target. |
| `frontend/app/store/keymodel.ts` | Keyboard shortcut registration. `Ctrl/Cmd +/-/0` mapped to `zoomIn/Out/Reset`. |
| `frontend/app/view/term/termViewModel.ts` | `termZoomAtom` (reads `term:zoom` from block metadata), `fontSizeAtom` (base * zoom). |
| `frontend/app/view/term/term.tsx` | Terminal construction useEffect (line 110-172) and font-size-only update useEffect (line 176-182). |
| `frontend/app/element/zoomindicator.tsx` | Transient zoom % overlay. Reads `zoomIndicatorVisibleAtom` and `zoomIndicatorTextAtom`. |
| `frontend/wave.ts` | Startup init. Pins `setZoomFactor(1.0)`, calls `initChromeZoom()`. |
| `frontend/app/block/blockframe.tsx` | Renders `data-blockid` attribute on pane container (line 677), used by wheel handler for target detection. |
| `frontend/tailwindsetup.css` | Declares `:root { --zoomfactor: 1; }` default. |
| `frontend/app/window/window-header.scss` | Header container. Uses `calc(Npx * var(--zoomfactor))` for height and font-size. |
| `frontend/app/statusbar/StatusBar.scss` | Status bar container. Uses `calc(Npx * var(--zoomfactor))` for height, font-size, version font-size. |
| `frontend/app/tab/tab.scss` | Individual tab styling. All values hardcoded px — does NOT respond to `--zoomfactor`. |
| `frontend/app/tab/tabbar.scss` | Tab bar container and add-tab button. All values hardcoded px. |
| `frontend/app/window/action-widgets.scss` | Window action widgets (right side of header). Hardcoded px. |
