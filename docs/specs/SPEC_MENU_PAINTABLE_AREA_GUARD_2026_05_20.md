# SPEC: Menu positioning framework — offset every menu into the paintable area

**Date:** 2026-05-20 (revised 2026-05-21)
**Author:** AgentX
**Status:** Draft

---

## TL;DR

A menu **cannot** render outside the window — the OS clips at the window frame, that state does not exist. The real failure is subtler: a menu gets *positioned* such that part of its body falls **outside the paintable area** — past a window edge, or behind a native browser-pane window — where it is clipped and unreachable.

The fix is always the same single operation: **offset the menu** so its entire body lands inside the paintable area. Today that offset is computed inconsistently — one menu does it correctly, most don't, across three unrelated code paths.

This spec defines **one positioning framework** (`useMenuPosition`) that every DOM menu routes through, and a guard that keeps it that way.

---

## 1. The behavior we want

A menu has:
- an **anchor** — an element rect, or a raw click point
- a **natural size** — measured once it renders

The framework's only job:

> If placing the menu at its anchor would push any part of it outside the **paintable area**, choose an **offset** (shift along an axis, and/or flip to the opposite side of the anchor) so the *entire* menu body lands inside the paintable area.

That's it. Nothing renders outside the window — impossible by construction. The framework picks the offset that makes the menu paint fully, correctly, inside the visible region.

---

## 2. The paintable area

> **Paintable area** — the window client region where a DOM-rendered menu is actually visible. It is the window viewport **minus** the rectangles occupied by **native child windows** that paint above the DOM.

Two kinds of boundary, treated identically:

1. **Window edge** — the menu must stay within `[0, innerWidth] × [0, innerHeight]`.
2. **Native-pane edge** — AgentMux browser panes are `CefBrowserView` instances: native OS child windows, **not** iframes ([`SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md`](SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md)). Native child windows paint **above** the host webview's DOM. A DOM menu whose body overlaps a browser pane is drawn *behind* that pane — invisible, even though its x/y is "inside the window."

A correct offset respects **both** boundaries. A framework that only checks `window.innerWidth/innerHeight` handles the first and silently fails the second.

> The window edge is the common case. The native-pane edge is the AgentMux-specific case that nothing in the codebase handles today.

---

## 3. Current state — how each menu offsets (or doesn't)

The Explore pass over the codebase (verified against HEAD `7e3d5722`) found **7+ menu surfaces** and **three unrelated positioning strategies**:

| # | Surface | File | Render layer | Offset logic | Correct? |
|---|---|---|---|---|---|
| 1 | **FlyoutMenu** (hamburger, dropdowns) | `frontend/app/element/flyoutmenu.tsx:51` | DOM | floating-ui `computePosition`, **placement only — no middleware** | ❌ never offsets |
| 2 | **MenuButton** | `frontend/app/element/menubutton.tsx` | DOM | wraps FlyoutMenu | ❌ inherits |
| 3 | **FlyoutMenu submenu** | `flyoutmenu.tsx:262-289` | DOM | ad-hoc manual `getBoundingClientRect` vs `innerWidth/Height`, one-shot `flipped` flag | ⚠️ offsets, but window-edge only + buggy (see §3.1) |
| 4 | **Popover** (status bar, notifications) | `frontend/app/element/popover.tsx:59` | DOM | floating-ui, `offset` only — `shift`/`flip` only if the *caller* passes them | ❌ no offset by default |
| 5 | **Tooltip** | `frontend/app/element/tooltip.tsx:49` | DOM | floating-ui `offset(10)` + `flip()` + `shift({padding:12})` | ✅ offsets correctly (window edge only) |
| 6 | **More dropdown** (widget bar) | `frontend/app/window/action-widgets.tsx:229` | DOM `position:fixed` | manual `window.innerWidth - rect.right` | ⚠️ horizontal offset only |
| 7 | **TokenBreakdownPopover** | `frontend/app/statusbar/TokenBreakdownPopover.tsx:76-84` | DOM `position:fixed` | manual clamp, bespoke 8px gutter | ⚠️ correct but hand-rolled |
| 8 | **Native context menus** (all right-click) | `frontend/app/store/contextmenu.ts:52-58` | **native CEF/OS** | raw `clientX/clientY` → `getApi().showContextMenu` | ✅ OS offsets natively, paints above panes |
| 9 | **Command palette** | `frontend/app/modals/command-palette.tsx` | DOM modal | modal-v2 centering | ✅ centered, never near an edge |

Of the six **DOM menus**, exactly **one** (Tooltip) offsets correctly — and even that one only knows about the window edge, not native panes.

### 3.1 Confirmed code facts

- `flyoutmenu.tsx:51-54` — `computePosition(referenceEl, floatingEl, { placement: props.placement ?? "bottom-start" })`. No middleware array at all. The most-used DOM menu computes a raw placement and never offsets.
- `popover.tsx:59` — `middleware = [...(props.middleware ?? []), offsetMiddleware(offsetVal)]`. `shift`/`flip` exist only if a caller threads them in. `NotificationPopover` does not.
- `flyoutmenu.tsx:262-289` — the `SubMenu` component *does* offset (`overflowRight = rect.right - window.innerWidth`, flip-left / clamp-top), proving menus genuinely need offsetting — but: it's window-edge-only, and gated by a one-shot `flipped` flag that never re-evaluates after a resize or scroll.
- `contextmenu.ts:56` — `{ x: clientX, y: clientY }` handed to native `showContextMenu`. No DOM-side offset, and none needed: the OS positions native menus and paints them above everything, native panes included.

The submenu code at §3.1 line 3 is the proof point: a developer hit a menu landing past the window edge and hand-patched an offset for that *one* surface. The framework generalizes that patch and extends it to the native-pane boundary.

---

## 4. Why it's inconsistent

| Strategy | Surfaces | Mechanism |
|---|---|---|
| **floating-ui** | FlyoutMenu, MenuButton, Popover, Tooltip, NotificationPopover | `@floating-ui/dom` `computePosition` |
| **Manual viewport math** | More dropdown, TokenBreakdownPopover, FlyoutMenu submenu | hand-rolled `getBoundingClientRect` + `innerWidth/Height` |
| **Native OS** | all right-click context menus | CEF `showContextMenu` → OS menu |

Nothing *enforces* a single path. `@floating-ui/dom` is already a dependency and already has the exact middleware needed (`shift`, `flip`, `size`) — but four of the five floating-ui consumers don't apply it. Each new menu re-decides positioning from scratch.

**Unification today: 2/5.** Target: **4/5** — every DOM menu through one offsetter; native context menus stay native (that's correct, not a gap).

---

## 5. The framework — `useMenuPosition`

One hook. Every **DOM** menu routes through it. Native context menus are out of scope (§7).

```typescript
interface MenuPositionRequest {
    anchor: HTMLElement | DOMRect | { x: number; y: number };
    placement?: Placement;        // preferred side; flips if it doesn't fit
    gutter?: number;              // min gap from any paintable edge (default 8)
    avoidNativePanes?: boolean;   // default true — treat browser-pane rects as boundaries
}

interface MenuPositionResult {
    style: JSX.CSSProperties;     // position:fixed; left/top — the offset, applied
    placement: Placement;         // side chosen after any flip
    maxHeight: number;            // when the menu is taller than the free space
    maxWidth: number;
}

function useMenuPosition(req: () => MenuPositionRequest): () => MenuPositionResult;
```

### 5.1 How it computes the offset

1. **Resolve anchor** to a `DOMRect` (element → `getBoundingClientRect`; point → zero-size rect).
2. **Measure** the menu's natural size.
3. **Compute the paintable area** — viewport rect minus native-pane rects (when `avoidNativePanes`). See §5.3.
4. **Offset** via floating-ui with a *fixed, non-optional* middleware stack:
   - `offset(gutter)` — the gap from the anchor
   - `flip()` — if the preferred side doesn't fit, use the opposite side of the anchor
   - `shift({ padding: gutter })` — slide the menu along the cross-axis to pull it back inside
   - `size()` — when the menu is taller/wider than the free space, emit `maxHeight`/`maxWidth` so it scrolls internally instead of being placed partly outside
   - **custom `boundary`** — floating-ui's `detectOverflow` boundary is set to the **paintable area**, not the default viewport. This is the line that makes `flip`/`shift` offset away from native panes, not just window edges.
5. **Residual check** — if the resolved rect still intersects a native-pane rect (floating-ui can't always satisfy a punched-out region), apply §5.2.
6. Return the offset as `style`, plus the resolved `placement` and size caps.

### 5.2 When the menu can't fully fit

A menu anchored dead-center over a maximized browser pane may have no fully-paintable spot. Degrade in this order:

1. **Shrink** — apply `maxHeight`/`maxWidth` + internal scroll so the menu fits a non-occluded strip. Covers the overwhelming majority.
2. **Re-anchor** — move the menu to the nearest fully-paintable region, draw a connector back to the anchor.
3. **Promote to native** — render as a native OS menu via the `contextmenu.ts` path, which paints above browser panes. Only possible for menus expressible as `ContextMenuItem[]` (no custom JSX rows).

**v1 ships shrink-only (step 1).** Steps 2-3 are follow-ups — see Q2.

### 5.3 Sourcing native-pane rectangles

The layout reducer already knows every browser pane's `blockId` and on-screen rect — `browser_pane_resize` propagates layout changes to the HWND. A `getNativePaneRects(): DOMRect[]` selector walks the current tab's layout tree for blocks with `view: "browser"` (+ any future native-HWND view) and returns their client rects. Measured once per menu-open, same frame.

---

## 6. The guard

`useMenuPosition` is the carrot. The guard makes a wrongly-offset menu a **caught error**, not a silent visual bug.

### 6.1 Dev-mode runtime assertion

`assertMenuInPaintableArea(el: HTMLElement)`, called from each DOM menu's mount effect under `AGENTMUX_DEV=1`:

- One RAF after the menu opens, measure its rect.
- Intersect with the paintable area.
- Any part outside, or behind a native pane → `console.error` tagged `[menu-guard]` with the anchor and the offending boundary. Surfaces via `muxlog host '[menu-guard]'`.
- Dev-only — stripped from release builds, zero runtime cost.

### 6.2 CI grep gate

- Flag any `computePosition(` in `frontend/app/**` outside `useMenuPosition` / the three sanctioned components.
- Flag new `position: fixed` + `getBoundingClientRect` offset arithmetic outside the framework.
- Forces new menus through the hook instead of re-rolling manual math — the thing that produced today's 2/5.

### 6.3 Out of scope for the guard

Native context menus. The OS offsets and paints those; they are correctly exempt. The guard governs **DOM menus only**.

---

## 7. What converges, what stays native

| Surface | Action |
|---|---|
| Native context menus (all right-click) | **Keep native.** Correct as-is — OS offsets them, they paint above browser panes. |
| FlyoutMenu | Route through `useMenuPosition`; delete the ad-hoc submenu offset (§3.1). |
| MenuButton | No change — inherits the FlyoutMenu fix. |
| Popover | Route through `useMenuPosition`; make `flip`/`shift`/`size` non-optional. |
| Tooltip | Migrate for consistency — already correct, low priority (Q3). |
| More dropdown | Replace manual `innerWidth - rect.right` with `useMenuPosition`. |
| TokenBreakdownPopover | Replace bespoke clamp with `useMenuPosition`. |
| Command palette | No change — modal-centered, never near a boundary. |

Result: **one offsetter for every DOM menu; native menus stay native.** A literal 5/5 ("one component for menus, tooltips, modals, and OS context menus") isn't worth collapsing genuinely different surfaces — 4/5 is the right target.

---

## 8. Implementation phases

### Phase 0 — confirm unknowns (½ day)
Q1 (editor pane DOM-vs-native), Q2 (fallback scope), verify `getNativePaneRects` sources cleanly from the layout reducer.

### Phase 1 — `useMenuPosition` + paintable-area selector (1.5 days)
`frontend/app/util/menu-position.ts` — the hook, `getPaintableArea()`, `getNativePaneRects()`. Fixed middleware stack with custom `boundary`. Unit tests: anchor at each edge, anchor over a synthetic native-pane rect, tall-menu shrink.

### Phase 2 — migrate FlyoutMenu + Popover (1 day)
Covers the majority of DOM menus (MenuButton, NotificationPopover inherit). Delete the one-shot `flipped` submenu logic — submenus offset through the hook.

### Phase 3 — migrate manual-math surfaces (½ day)
More dropdown, TokenBreakdownPopover → `useMenuPosition`. Optional: Tooltip.

### Phase 4 — the guard (½ day)
`assertMenuInPaintableArea` dev assertion + CI grep gate.

### Phase 5 — edge-case pass (½ day)
Manual matrix per §9.

**Total: ~5 days.**

---

## 9. Validation

- [ ] `npm run build` green
- [ ] Unit tests: `useMenuPosition` offsets correctly at all 4 window edges; offsets away from a synthetic native-pane rect; shrinks a too-tall menu
- [ ] `task dev` manual matrix:
  - [ ] Hamburger menu with the window dragged so ≡ is near the right edge → menu flips/shifts fully in-bounds
  - [ ] `FlyoutMenu` submenu near the bottom edge → offsets up, stays whole
  - [ ] A DOM menu (More dropdown) anchored where it would overlap a maximized browser pane → menu is offset/shrunk, never drawn behind the pane
  - [ ] Window resized very small → no menu placed partly past a window edge
  - [ ] Right-click bottom-right pane → native context menu fully visible (sanity — native path, unchanged)
- [ ] `muxlog host '[menu-guard]'` shows zero violations across the matrix
- [ ] CI grep gate fails a deliberately-added stray `computePosition` call

---

## 10. Risk register

| Risk | Mitigation |
|---|---|
| Paintable area minus N panes is not a single rect; floating-ui `boundary` wants a rect | Pass the largest inscribed free rect as `boundary`, then run the §5.1-step-5 residual check for leftover overlap |
| `getNativePaneRects` reads stale geometry | Source from the layout reducer's current state, measured the same frame the menu opens; `autoUpdate` re-measures on RAF |
| Migration regresses a working menu | Phase-by-phase, each phase independently shippable; Tooltip migration is optional precisely because it already works |
| Dev assertion log noise | `[menu-guard]` tag, dev-only, debounced to one log per violation per open |
| Perf — measuring the paintable area on every open | N panes per tab rarely exceeds ~6; compute once per open, not per RAF |

---

## 11. Open questions

1. **Q1** — Editor panes: DOM (Monaco/CodeMirror in-webview) or native HWND? Determines whether they're boundaries. The 2026 monaco-removal work suggests DOM — confirm.
2. **Q2** — Fallback ladder (§5.2): ship shrink-only for v1, or build re-anchor + native-promote now? Recommend shrink-only; file the rest.
3. **Q3** — Migrate Tooltip in Phase 3 or leave it independent? It works; migration is purely for one-offsetter consistency.
4. **Q4** — Window straddling two monitors at different DPI: is `window.innerWidth/innerHeight` still the right paintable bound? Likely yes (window-relative, not screen-relative) — verify on the Phase 5 matrix.
5. **Q5** — Does any menu *intentionally* sit flush to a boundary today? Audit before enforcing the guard so a deliberate design isn't "corrected."

---

*End of spec.*
