# Auto-discovery pane-overlay clipping (declarative `data-pane-overlay`)

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-11
**Driving observation:** The browser pane is a native Win32 HWND that paints above DOM regardless of CSS z-index — the "airspace problem." We already solve this for modals via `frontend/app/platform/pane-overlay.ts` + `browser_panes_set_overlay_clip` IPC + host-side `SetWindowRgn(hwnd, RGN_DIFF, ...)`. Menus (FlyoutMenu / `ContextMenuModel.showContextMenu`) don't register their rects with that mechanism, so they paint *under* the browser pane unless a modal is concurrently open. Symptom: "menu over browser pane sometimes works, sometimes the browser leaks through" — a registration gap.

We need an architecture where any new DOM overlay automatically participates in clipping, without each callsite having to remember to call a hook. Proposes a `data-pane-overlay` attribute as the declarative contract.

---

## 1. The class of bugs we're closing

Today, an overlay needs to call `usePaneOverlay()` to register its rect. Future overlays (popovers, tooltips, command-palette transient surfaces, sub-menus from #792) are at risk every time someone forgets. The class:

```
Any DOM element that paints above the browser pane in CSS z-order, but
that the CEF compositor doesn't know about → renders BELOW the browser
pane in the actual Win32 paint order.
```

Examples in the codebase today:
- `frontend/app/element/flyoutmenu.tsx` (hamburger menu + submenus) — **not registered**
- `frontend/app/store/contextmenu.tsx` (right-click menus on the tab bar empty space) — **not registered**
- `frontend/app/element/tooltip.tsx` — **probably not registered**, may not have been noticed
- `frontend/app/element/popover.tsx` — same
- `frontend/app/modals/*` — **registered correctly** via `usePaneOverlay()` today

Each callsite is a place where the bug can recur on the next refactor.

---

## 2. Proposal: declarative attribute

Any DOM element that participates in pane clipping declares itself via a single attribute:

```tsx
<div data-pane-overlay>
    {/* anything that needs to clip the browser pane */}
</div>
```

A central service watches for the attribute, measures each tagged element, dispatches the union to the host, and re-measures on changes. No per-component imports, no hooks to remember, no lifecycle bookkeeping.

### Why an attribute, not a hook

| Aspect | `usePaneOverlay()` hook | `data-pane-overlay` attribute |
|---|---|---|
| Discoverability | Need to read each component to know if it registers | One grep finds every overlay |
| Forgetting | Easy — silent visual bug only on Windows + browser pane | Hard — visible diff in JSX |
| Lifecycle correctness | Each component handles cleanup | Centralized — observer handles mount/unmount |
| Resize tracking | Each component owns ResizeObserver | Shared — one observer for all overlays |
| Cost | One closure + listener per overlay | One observer + one map for all overlays |
| Composability with existing modal code | Modals keep `usePaneOverlay`, no migration forced | Modals keep `usePaneOverlay`, optional migration |

The attribute is also self-documenting: a code reader sees `data-pane-overlay` and immediately knows the element participates in browser-pane clipping, without context-switching to a hook implementation.

---

## 3. The service

New module: `frontend/app/platform/pane-overlay-auto.ts`. Singleton, initialized once at app startup (in `app-init.ts` or equivalent).

```ts
type RectKey = string;  // generated per-element via WeakMap

const rects = new Map<RectKey, DOMRectReadOnly>();
const elementToKey = new WeakMap<Element, RectKey>();
const tracked = new WeakSet<Element>();
let nextKey = 0;

const SELECTOR = "[data-pane-overlay]";

function startService(): void {
    // 1. Watch the document subtree for added/removed [data-pane-overlay] elements.
    const mo = new MutationObserver((muts) => {
        let changed = false;
        for (const m of muts) {
            for (const node of m.addedNodes) {
                if (node instanceof Element) changed = registerSubtree(node) || changed;
            }
            for (const node of m.removedNodes) {
                if (node instanceof Element) changed = unregisterSubtree(node) || changed;
            }
            if (m.type === "attributes" && m.target instanceof Element) {
                // attribute toggle
                if (m.target.hasAttribute("data-pane-overlay")) {
                    changed = register(m.target) || changed;
                } else {
                    changed = unregister(m.target) || changed;
                }
            }
        }
        if (changed) dispatch();
    });
    mo.observe(document.body, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ["data-pane-overlay"],
    });

    // 2. Bootstrap: any tagged element already in the DOM at startup
    for (const el of document.querySelectorAll(SELECTOR)) register(el);
    dispatch();

    // 3. Window resize / scroll re-measures every tracked rect.
    const reMeasureAll = () => {
        let changed = false;
        for (const el of [...tracked]) {
            if (!document.body.contains(el)) {
                unregister(el);
                changed = true;
            } else {
                changed = updateRect(el) || changed;
            }
        }
        if (changed) dispatch();
    };
    window.addEventListener("resize", reMeasureAll, { passive: true });
    window.addEventListener("scroll", reMeasureAll, { passive: true, capture: true });
}

function register(el: Element): boolean {
    if (tracked.has(el)) return updateRect(el);
    tracked.add(el);
    const key = `o${nextKey++}`;
    elementToKey.set(el, key);

    // Per-element ResizeObserver so internal size changes (submenu items
    // streaming in, menu width changing) re-measure without polling.
    const ro = new ResizeObserver(() => {
        if (updateRect(el)) dispatch();
    });
    ro.observe(el);
    // Stash the observer on the element so unregister can disconnect.
    (el as any).__poaObserver = ro;

    updateRect(el);
    return true;
}

function unregister(el: Element): boolean {
    if (!tracked.has(el)) return false;
    tracked.delete(el);
    const key = elementToKey.get(el);
    if (key) {
        elementToKey.delete(el);
        rects.delete(key);
    }
    (el as any).__poaObserver?.disconnect();
    delete (el as any).__poaObserver;
    return true;
}

function updateRect(el: Element): boolean {
    const key = elementToKey.get(el);
    if (!key) return false;
    const r = el.getBoundingClientRect();
    const prev = rects.get(key);
    if (
        prev &&
        prev.left === r.left && prev.top === r.top &&
        prev.right === r.right && prev.bottom === r.bottom
    ) return false;
    rects.set(key, r);
    return true;
}

function dispatch(): void {
    const flatRects = [...rects.values()].map((r) => ({
        x: Math.floor(r.left),
        y: Math.floor(r.top),
        width: Math.ceil(r.width),
        height: Math.ceil(r.height),
    }));
    invokeCommand("browser_panes_set_overlay_clip", { rects: flatRects }).catch(() => {
        // swallow — IPC failures don't break frontend logic
    });
}

function registerSubtree(root: Element): boolean {
    let changed = false;
    if (root.matches(SELECTOR)) changed = register(root) || changed;
    for (const el of root.querySelectorAll(SELECTOR)) changed = register(el) || changed;
    return changed;
}

function unregisterSubtree(root: Element): boolean {
    let changed = false;
    if (tracked.has(root)) changed = unregister(root) || changed;
    for (const el of root.querySelectorAll(SELECTOR)) changed = unregister(el) || changed;
    return changed;
}
```

`startService()` called once from `frontend/app-init.ts` after the DOM is ready.

### IPC reuse

No new host API. `browser_panes_set_overlay_clip` already exists and dispatches `SetWindowRgn(hwnd, RGN_DIFF, ...)`. We just feed it the auto-discovered rect set in addition to whatever modal/etc. callers send.

**Open question:** today `pane-overlay.ts` sends one set; if the auto service sends another, the host's last-writer-wins. The two should merge. Simplest fix: route both paths through the auto service's map. The legacy `usePaneOverlay()` hook becomes a thin wrapper that registers a rect by id in the same map. Existing modal callers keep working; the auto-attribute path supplements them.

---

## 4. Migration path

Drop-in per overlay component. No interface change for users.

### FlyoutMenu (`frontend/app/element/flyoutmenu.tsx`)

```tsx
// Main menu wrapper
<Portal>
    <div
        class={clsx("menu", props.className)}
        ref={registerFloating}
        style={floatingStyle()}
        data-pane-overlay   // ← add
    >
```

```tsx
// SubMenu wrapper
<div
    ref={...}
    class="menu sub-menu"
    style={...}
    data-pane-overlay      // ← add
>
```

That's the entire change for the hamburger menu and submenus — including the radio Theme/Opacity lists.

### ContextMenuModel (`frontend/app/store/contextmenu.tsx` or wherever the right-click menu renders)

Same: add `data-pane-overlay` to the rendered context-menu container.

### Tooltip / Popover

If they render to a Portal and have any chance of overlapping a browser pane (they do), tag them. One-line change each.

### Modal

Already uses `usePaneOverlay()`. Two options:
- Leave as-is (the legacy hook continues to feed the same map)
- Migrate to attribute — `data-pane-overlay` on the modal root; remove the hook call. Net loss of LOC.

---

## 5. Edge cases

- **Empty rect set.** When the last overlay unmounts, `rects` is empty; we dispatch `{ rects: [] }`. The host calls `SetWindowRgn(hwnd, NULL)` and the pane goes fully visible. (Existing behavior, preserved.)
- **Element moves without ResizeObserver firing.** Scroll, transform, position changes don't trigger ResizeObserver. The window-level scroll listener covers scroll; for arbitrary transforms (e.g., a draggable popover), the caller can either skip the data-attribute path or call `requestAnimationFrame(reMeasureAll)` manually. Submenus in the current FlyoutMenu don't transform after mount, so this is not a concern for #792.
- **High-frequency updates.** A streaming menu (items added per token) would emit many `dispatch()` calls. Mitigate via `requestAnimationFrame` coalescing in `dispatch()` so at most one IPC per frame.
- **Devtools / DOM inspector toggling the attribute.** Mutation observer correctly handles attribute add/remove — no special case needed.
- **`display: none` element.** `getBoundingClientRect()` returns a zero rect. Filter those out in `dispatch()`: `if (r.width > 0 && r.height > 0)`. Otherwise we'd send a zero rect that does nothing useful but adds host-side cycles.
- **Element nested inside another overlay.** Both get registered; both rects ship to the host. The host unions them via successive `RGN_DIFF` ops, so the inner overlay's rect is redundant but harmless. Could optimize later by filtering descendants, but not in v1.
- **iframe / shadow DOM overlays.** The MutationObserver on `document.body` doesn't cross shadow roots. We don't currently render menus inside shadow roots; if that changes, the service needs to re-target. Document the assumption.

---

## 6. Test plan

- [ ] **Smoke:** open the hamburger menu over a browser pane — the menu fully covers the pane, no leak-through.
- [ ] **Smoke:** open Theme submenu, then Opacity submenu — both clipped correctly during the hop.
- [ ] **Smoke:** open a context menu (right-click empty tab-bar space) over a browser pane — same.
- [ ] **Smoke:** resize the window while a menu is open — clipping updates.
- [ ] **Smoke:** scroll while a tooltip is open — clipping follows.
- [ ] **Existing-modal regression:** any modal that already uses `usePaneOverlay()` continues to work.
- [ ] **Unit test:** `pane-overlay-auto.ts` registers/unregisters on attribute add/remove, dispatches the right rect set.
- [ ] **Unit test:** zero-rect elements (display:none) are filtered out.
- [ ] **Perf:** continuous CPU while a menu is open is ≤ existing baseline (the service is event-driven, not polling).

---

## 7. Out of scope

- **Cross-monitor coordinate transforms** (DPI differences between monitors). Host already handles this for `browser_pane_resize`; the same path applies here.
- **Hiding the browser pane entirely while any overlay is open** (the "nuclear option" in `SPEC_BROWSER_PANE_Z_ORDER`). The auto-clip approach is finer-grained and Just Works.
- **macOS / Linux equivalence.** CEF on those platforms doesn't have the same airspace problem (different compositor model). The IPC is a no-op there; service registration is harmless.

---

## 8. Effort

| Component | LOC | Notes |
|---|---|---|
| `pane-overlay-auto.ts` service module | ~150 | One-time write |
| Bootstrap in `app-init.ts` | ~3 | Single call |
| Migrate `usePaneOverlay()` to feed the same map | ~30 | Or skip; both paths can coexist |
| `data-pane-overlay` on FlyoutMenu (main + SubMenu) | ~4 | Two attribute additions |
| `data-pane-overlay` on ContextMenuModel | ~2 | One attribute addition |
| `data-pane-overlay` on tooltip / popover | ~4 | Two attribute additions |
| Unit tests for the service | ~80 | Mount/unmount/resize/scroll, rect filtering |
| Smoke test plan in PR description | — | ~10 min manual |
| **Total** | **~270** | **~1 day** |

---

## 9. Recommended PR bundling

Two options:

**A — Bundle with PR #792 (the snappy menu).** The hamburger + submenus from #792 are the most visible new overlays affected. Adding `data-pane-overlay` to them at the same time as the snappy refactor means the new menu opens fast AND clips correctly. Risk: scope creep — #792 was "perf only," and the clip system is a separate architectural concern.

**B — Separate PR after #792 merges.** Cleaner separation. #792 stays narrowly scoped. The auto-clip PR touches FlyoutMenu surgically (just adds attributes), context menu, tooltips, modals.

**Recommendation: B.** #792 is already in review and rebases are expensive; landing it as-is and following with the auto-clip PR keeps blast radius small. The clip-leak bug is real but not net-new from #792 — it existed before. The spec rides with #792 (so reviewers see the plan), implementation follows on its own branch.

---

## 10. Cross-references

- Existing modal clip path: `frontend/app/platform/pane-overlay.ts`
- IPC dispatcher: `agentmux-cef/src/ipc.rs` line 381+
- Host-side region math: `agentmux-cef/src/browser_panes.rs` line 396-472
- Airspace problem doc: `specs/SPEC_BROWSER_PANE_Z_ORDER_2026_04_21.md`
- Modal clip spec: `docs/specs/SPEC_MODAL_PANE_CLIP_2026_04_24.md`
- Snappy menu PR (companion): #792
- Discussion #707 (reducer-stack ongoing thread) — append a pointer to this spec if/when implemented, since it touches the "what *isn't* in the reducer" boundary.

---

## 11. Driving observation (verbatim)

> "the persistent problem we have are menu items (whether from hamburger or empty-space right click on top bar) the browser DOM section of the browser pane will be atop the menu .. it gets fixed sometimes, but then breaks again. what's a strategy to keep it right?"
