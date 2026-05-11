# FlyoutMenu hover-delay forensics + remediation plan

**Date:** 2026-05-10
**Owner:** AgentA
**Trigger:** User reported "unusual delay when hovering through items that does not appear in VSCode" on the hamburger menu (Theme / Opacity submenus on PR #791).
**Verdict:** Real, ~16–18ms perceived latency per submenu open, plus per-sibling Portal churn. Native menus are instant because they avoid every async hop that this implementation introduces.

This report walks the existing render pipeline, names the specific lines that cost user-perceived latency, and proposes a stepwise remediation that keeps the public API of `FlyoutMenu` stable.

---

## 1. Where the delay comes from — the hover-to-paint timeline

File: `frontend/app/element/flyoutmenu.tsx`

```
T+0   ms  mouseenter fires on parent .menu-item
          handleMouseEnterItem runs (line ~112)
            stopPropagation
            setVisibleSubMenus  (O(N) walk over the map, lines 122-144)
            setHoveredItems
            handleSubMenuPosition(...) (line 154)
              ──► itemRect.getBoundingClientRect()       (sync)
              ──► setTimeout(0, () => { … })             ⏸ defers to next macrotask
T+1-3 ms  Solid render pass 1
            <For> reconciles
            isActive() memos recompute for all items
            <Show when={visible && subItems}> → SubMenu mounts
              <Portal>                                    ── new DOM tree to <body>
                <div class="sub-menu"
                     style={visibility: hidden ⚠}>        ── still hidden
T+4-15 ms (idle — waiting for setTimeout(0) macrotask boundary)
T+16  ms  setTimeout callback fires
            reads subMenuRef.offsetWidth/Height
            setSubMenuPosition(...)
T+17  ms  Solid render pass 2
            <SubMenu> re-renders
            position() now resolved → isPositioned() true → visibility: visible
T+18  ms  browser paints visible submenu          ← user finally sees it
```

That's a **missed frame at 60 Hz**, and on top of it every sibling switch (Theme → Opacity) unmounts the previous submenu's Portal and runs the whole sequence again.

VSCode submenus appear "instant" because they:
- compute position **synchronously** before painting,
- keep one shared overlay alive (no Portal per submenu, no `visibility: hidden` two-render bootstrap),
- swap visibility on existing DOM rather than mount/unmount.

---

## 2. Root causes, ranked by user-perceived impact

| # | Cause | Where | Cost per hover |
|---|---|---|---|
| 1 | **`setTimeout(0)` in `handleSubMenuPosition`** | flyoutmenu.tsx:84 | One full frame (≈16ms) deferred before visibility flip — single biggest source |
| 2 | **Per-sibling Portal unmount/remount** | flyoutmenu.tsx:225 (the `<Show>` around `<SubMenu>`) | ~5ms of Solid teardown + create + ref assignment + initial hidden render every time the user moves between Theme and Opacity |
| 3 | **`visibility: hidden` two-render bootstrap** | flyoutmenu.tsx:272-275, :286 | Forces a render where the submenu is in the DOM but invisible, then a second render to reveal it. Tied to (1). |
| 4 | **`autoUpdate` polling at RAF rate** | flyoutmenu.tsx:57 | Continuous `computePosition` measurement while the menu is open. Designed for moving anchor elements; the hamburger button never moves. |
| 5 | **`visibleSubMenus` O(N) ancestor walk on every hover** | flyoutmenu.tsx:124-141 | One sweep over the whole visibility map per hover; ~10 iterations × handful of memo reads. Small absolute cost but compounding. |
| 6 | **String-split ancestor reconstruction** | flyoutmenu.tsx:126-129, 146-149 | Rebuilds `["0", "0-4", "0-4-2", "0-4-2-1"]` from `"0-4-2-1"` on every hover. Negligible alone, but adds friction. |
| 7 | **All `isActive()` memos invalidated on hover** | flyoutmenu.tsx:183 | `hoveredItems()` is a single signal; flipping it re-evaluates `isActive` for every item. Cheap but proportional to item count. |

Causes 1–3 are tightly coupled and produce the *single* "pop after a frame" feeling. Cause 4 keeps the layout engine busy while the menu is idle.

---

## 3. Why the existing pattern was chosen (and what to keep)

The current architecture is not arbitrary:
- **Portal per submenu** escapes any ancestor `overflow: hidden` / `transform` / `contain: paint`. That escape is real and valuable.
- **`setTimeout(0)` before positioning** is a common workaround when measurements need DOM to be in the tree — except Solid mounts synchronously, so the workaround predates this stack.
- **`autoUpdate`** is the right call when the trigger moves (drag-attached menus). The hamburger trigger is fixed; the polling is gratuitous here.
- **`visibility: hidden`** prevents a one-frame flash at `top: 0; left: 0`. The fix is to compute position before paint, not to mask it.

The remediation keeps the Portal escape (so future overflow-clipped contexts still work) but removes everything else.

---

## 4. Remediation plan — ordered by impact / effort

Each phase is independently shippable and additive.

### Phase 1 — Eliminate the macrotask hop (highest impact, ~30 LOC)

Replace `setTimeout(0)` in `handleSubMenuPosition` with synchronous measurement.

**Why this works:** by the time `handleMouseEnterItem` runs, Solid will *synchronously* mount the SubMenu Portal in response to `setVisibleSubMenus`. The submenu DOM exists in the document by the next microtask. We can measure it inside a `queueMicrotask` (or `requestAnimationFrame` for a single-frame wait that still beats macrotask scheduling) without the visibility detour.

Better: measure with the parent item's rect + a `width: max-content` submenu so the dimensions are known from style alone. Position the submenu before render via the JSX:

```tsx
// proposed handleMouseEnterItem
const itemRect = (event.currentTarget as HTMLElement).getBoundingClientRect();
const scrollTop  = window.scrollY || document.documentElement.scrollTop;
const scrollLeft = window.scrollX || document.documentElement.scrollLeft;

// Position synchronously using the parent rect + viewport edge clamp.
// The submenu uses `width: max-content` so its own width does not need
// to be measured before placement; only the right-edge flip does.
const tentativeLeft = itemRect.right + scrollLeft - 2;
const top           = itemRect.top   + scrollTop  - 2;
const placement = { top, left: tentativeLeft, label: item.label };

setSubMenuPosition((prev) => ({ ...prev, [key]: placement }));
setVisibleSubMenus(/* ... */);
setHoveredItems(/* ... */);
```

After the initial mount, an `onMount` inside `SubMenu` does a single `getBoundingClientRect()` and applies the right-edge or bottom-edge flip if needed — but the *first* paint already lands at the correct or near-correct location with `visibility: visible`. No two-render dance.

Drops `visibility: hidden` entirely. Drops the macrotask boundary entirely.

### Phase 2 — Stop submenu churn between siblings (~50 LOC)

Currently `<Show when={visibleSubMenus()[key]?.visible && item.subItems}>` mounts and unmounts each submenu independently. Switching hover from Theme to Opacity rebuilds DOM.

Approach: render ALL submenus inside ONE shared overlay component that re-uses a single Portal. Each top-level item with `subItems` registers an "open submenu" intent, and the overlay renders only the currently active one, **but the overlay itself never unmounts**. Effectively:

```tsx
function FlyoutMenu(props) {
    const [activeSubKey, setActiveSubKey] = createSignal<string | null>(null);

    // ... existing top-level render ...

    return (
        <>
            {/* main menu */}
            <Portal>
                <Show when={activeSubKey() != null}>
                    <SubMenuShell
                        items={resolveSubItems(activeSubKey())}
                        rect={cachedItemRects[activeSubKey()]}
                    />
                </Show>
            </Portal>
        </>
    );
}
```

A single SubMenuShell Portal mounts on first hover and stays mounted; its contents (item list + position) update as the user hovers between siblings. No Solid `<Show>` teardown.

For deeper nesting (submenu-of-submenu), the same shell handles it via a stack: `activeSubKey()` is replaced with `activeSubPath: string[]`, and the shell renders one level per stack entry. All in one Portal.

### Phase 3 — Stop redundant `autoUpdate` (~5 LOC)

The hamburger button doesn't move while its menu is open. Replace `autoUpdate(referenceEl, floatingEl, updatePosition)` (continuous RAF polling) with a single `updatePosition()` call on open + a `resize` window listener for catastrophic layout shifts:

```tsx
const registerFloating = (el: HTMLElement) => {
    floatingEl = el;
    requestAnimationFrame(updatePosition);  // single shot
    window.addEventListener("resize", updatePosition, { passive: true });
};
```

This recovers per-frame measurement work — small per-frame but constant while the menu is open. Especially helpful when submenus are open and would otherwise have their parent re-measured 60 times per second.

### Phase 4 — Memoize the ancestor walk + drop string-split keys (~40 LOC)

Replace string-keyed dash-separated paths with explicit `MenuItem` instances having `parent` references built once at menu construction. Hover handlers then become:

```tsx
const handleMouseEnter = (item: MenuItem) => {
    setActivePath(item.ancestorChain);   // precomputed
    setOpenSubmenu(item.id);
};
```

No string-split per hover, no O(N) visibility-map walks.

This is also the right structural change to support eventually exposing typed `MenuItem` with `parent: MenuItem | null` and unit-testable path resolution.

### Phase 5 (optional) — Intent debounce against menu-chase (~20 LOC)

Native menus (macOS, VSCode) typically apply a **50–100ms intent debounce** before opening a submenu: the user must hover the parent long enough to indicate intent. This prevents "menu chase" — when the user moves diagonally toward a submenu, they cross over a sibling that would otherwise grab focus, snapping the wrong submenu open.

After phases 1–3 make individual submenu opens instant, a 50ms intent delay actually *improves* perceived smoothness on path-through hovers. Not a remedy for the current sluggishness, but a fit-and-finish improvement for after.

---

## 5. Recommended ship order

| Phase | Effort | Impact | Ship as |
|---|---|---|---|
| 1 — drop `setTimeout(0)` + `visibility: hidden` | 0.5 day | **High** (eliminates the 16ms macrotask hop) | Standalone PR; minimal blast radius. |
| 3 — replace `autoUpdate` with one-shot + resize listener | 0.25 day | Medium (recovers RAF cycles while menu open) | Bundle with Phase 1; same file. |
| 2 — single shared SubMenu shell | 1 day | High for users who hop between siblings | Separate PR; touches FlyoutMenu API surface enough to deserve isolated review. |
| 4 — drop string-split keys, explicit parent chain | 0.5 day | Low–Medium; mostly correctness + future-proofing | Bundled with Phase 2. |
| 5 — intent debounce | 0.25 day | Polish | Separate, after smoke. |

Phases 1 + 3 alone should erase the user-reported "unusual delay" symptom. Phases 2 + 4 reduce sustained sibling-switch cost and clean up the internals. Phase 5 is gravy.

---

## 6. Compatibility / risk notes

- **Portal escape stays.** Don't move submenu rendering out of `<Portal>`; some agent-pane panes have `contain: paint` ancestors that would clip in-DOM submenus.
- **`MenuItem` shape stays.** The fix is internal to FlyoutMenu; callers (`tabbar.tsx`, `data.tsx`) keep their existing arrays.
- **`checked` indicator** added on PR #791 continues to work as-is — it's pure CSS class, not affected by the rendering pipeline.
- **Submenu testing:** add a quick test that asserts (a) a single Portal exists in the DOM after sibling-switch, (b) `visibility: hidden` never appears as a transient inline style on the submenu container.

---

## 7. Cross-references

- Source: `frontend/app/element/flyoutmenu.tsx`, `flyoutmenu.scss`
- Caller: `frontend/app/tab/tabbar.tsx` (`tabBarMenuItems`)
- Submenu-using callers: `frontend/app/view/chat/data.tsx` (only other site with `subItems`)
- Related work: PR #791 (the Theme/Opacity hamburger menu that surfaced this) — the rendering issues exist regardless but were hidden when the menu only had top-level items
- Floating-UI library: `@floating-ui/dom` — `autoUpdate` docs at https://floating-ui.com/docs/autoUpdate
