# Menu snappiness — deep dive

**Date:** 2026-05-10
**Owner:** AgentA
**Branch:** `agenta/menu-snappy-research`
**Driving brief:** *"setTimeouts(0) are sloppy programming. Can we take a deep dive and figure out how to get [the menu] as snappy as possible."*

This continues from `docs/analysis/flyout-menu-hover-delay-2026-05-10.md` (which diagnosed and laid out a 5-phase remediation). Here we push further: the user's lossless framing is **zero perceived delay**, matching native context menus and VSCode. We catalogue every async hop, name what's avoidable, propose the architecture that opens submenus *before* the next paint, and pin a measurable target.

---

## 0. The target

Native Windows/macOS context menus and VSCode submenu open are perceived as **instant**. Concretely:

- **First paint of the submenu lands in the same frame as the `mouseenter` event** (≤ 16 ms after pointer movement at 60 Hz, ≤ 8 ms at 120 Hz).
- **No flicker.** No empty submenu drawn at `(0,0)` then repositioned.
- **No "menu chase".** Mouse traveling diagonally across siblings to reach the active submenu doesn't snap to intermediate items.

We're going to aim for the first two as a hard requirement and the third as a soft polish (Phase 5 in the earlier report; restated here).

The measurable target: **`pointermove`-to-`submenu visible` ≤ 1 animation frame budget** (16.67 ms at 60 Hz). Anything beyond that is regression.

---

## 1. Why `setTimeout(_, 0)` is the wrong tool, *everywhere*

`setTimeout(_, 0)` schedules a **macrotask**. Browser scheduling is:

```
[layout/paint] → [microtask queue (Promises, queueMicrotask)] → [rendering opportunity] → [macrotask] → ...
```

Concretely, on a 60 Hz display:

| Defer mechanism | Latency to fire | Survives paint? |
|---|---|---|
| Direct call (sync) | 0 ms | n/a |
| `queueMicrotask` | < 0.1 ms, before paint | No (runs before next paint) |
| `Promise.resolve().then` | < 0.1 ms, before paint | No |
| `requestAnimationFrame` | aligned to next paint, ~0–16 ms | Yes (fires *just before* paint) |
| **`setTimeout(_, 0)`** | **≥ 4 ms (browsers clamp), often 16 ms (post-paint)** | **Yes, but generally *after* the next paint** |
| `setTimeout(_, n)` for n > 0 | ≥ n ms, clamped | Yes |

`setTimeout(_, 0)` was historically a way to "yield to the browser" before the microtask queue existed. It now produces the **worst-case latency** for any of the deferral primitives — and crucially **places the callback after the next render**. That's exactly why the submenu flickers: it gets placed in the DOM with `visibility: hidden` *before* the paint that should have shown it, the user sees nothing, and the callback fires *after* the paint and triggers a second render in the next frame.

**Rule of thumb:**

- If the goal is "**run after the DOM exists but before the browser paints**" — use `queueMicrotask` (or, in Solid, just rely on the synchronous render: refs are populated before the JSX returns).
- If the goal is "**run aligned to the rendering pipeline**" (animation, scroll handlers, measurement-based layout) — use `requestAnimationFrame`.
- If the goal is "**defer truly low-priority work**" — use `requestIdleCallback`.
- `setTimeout(_, 0)` is **never** the right answer.

### 1.1 Roll call: every `setTimeout(_, 0)` in the frontend

| Site | What it defers | Real reason | Replacement |
|---|---|---|---|
| `frontend/app/element/flyoutmenu.tsx:84` (`handleSubMenuPosition`) | Submenu position calc + visibility flip | Wait for the submenu portal's DOM to exist so `offsetWidth/Height` can be read | Compute position synchronously from parent rect + `max-content` width (no measurement needed). If measurement IS needed, use `queueMicrotask` (fires before next paint, ~0ms). |
| `frontend/app/block/blockframe.tsx:148` | `.focus()` + `.select()` on a `ref` callback | Wait for element to be attached to the document before focusing | Solid `onMount` (fires after attachment) — synchronous, no macrotask hop |
| `frontend/app/view/agent/components/AgentSearchBar.tsx:58` | `.focus()` on a `ref` callback | Same as above | Same — Solid `onMount` |
| `frontend/app/store/command-source.test.ts:119` | `await new Promise(r => setTimeout(r, 0))` in a test | Wait for an async store update to settle | `await Promise.resolve()` flushes the microtask queue, or use a deterministic event-based wait. Test-only; not user-facing. |

The flyoutmenu site is the only one with user-visible impact, but the pattern shows up enough to warrant a codebase rule.

---

## 2. Hover-to-paint timeline, today and the goal

### 2.1 Today (PR #791 head)

```
T+0    mouseenter on parent item
       handleMouseEnterItem (sync)
         ├─ stopPropagation
         ├─ setVisibleSubMenus     (state write: signal A)
         ├─ setHoveredItems        (state write: signal B)
         └─ handleSubMenuPosition
              └─ setTimeout(_, 0)   ⏸ ENQUEUED → macrotask
T+1    Solid renders pass 1 (A + B fired)
         └─ <SubMenu> mounts in Portal with visibility: hidden
T+2    Browser paints frame N (submenu invisible)
T+16   Macrotask fires
         └─ setSubMenuPosition     (state write: signal C)
T+17   Solid renders pass 2
         └─ <SubMenu> re-renders with visibility: visible
T+33   Browser paints frame N+1 (submenu visible)            ← USER SEES IT
```

User-perceived delay: ~33 ms (worst case across 2 frames, > 1 frame). On a 30 Hz display or a busy main thread, easily 50+ ms.

### 2.2 Goal

```
T+0    mouseenter on parent item
       handleMouseEnterItem (sync)
         ├─ Read parent rect (synchronous, getBoundingClientRect)
         ├─ Compute submenu position from rect alone (no DOM measurement needed)
         ├─ Set state: { submenuOpen: keyOfItem, position: {top, left} }
         └─ (return)
T+1    Solid renders pass 1
         └─ <SubMenu> mounts at correct position, visibility: visible
T+2    Browser paints frame N — submenu visible          ← USER SEES IT
```

User-perceived delay: 0–2 ms, < 1 frame budget. Identical to native.

The trick: **don't measure the submenu before showing it.** Lock the submenu to `width: max-content` and use the parent's rect to position. The only measurement that needs the submenu actually in the DOM is the *edge-flip* (slide left or up if it would clip viewport). That can run synchronously *after* the first paint via `onMount` if needed — by the time the flip happens, the user has already seen the submenu at the primary position.

---

## 3. Architectural transforms

These are ordered by leverage. Each is independently shippable.

### 3.1 Synchronous position from parent rect (the headline win)

**Today:**

```ts
const handleSubMenuPosition = (key, itemRect, label) => {
    setTimeout(() => {
        const sub = subMenuRefs[key];
        if (!sub) return;
        const subW = sub.offsetWidth;
        const subH = sub.offsetHeight;
        // ... compute left/top with edge flip ...
        setSubMenuPosition((prev) => ({ ...prev, [key]: { top, left, label } }));
    }, 0);
};
```

**After:**

```ts
const handleSubMenuPosition = (key, itemRect, label) => {
    // Synchronous primary placement. No DOM measurement; the submenu
    // gets `width: max-content` from CSS, so its width is determined
    // by content alone — we don't need to read offsetWidth here.
    const scrollTop  = window.scrollY;
    const scrollLeft = window.scrollX;
    const top  = itemRect.top  + scrollTop  - 2;
    const left = itemRect.right + scrollLeft - 2;
    setSubMenuPosition((prev) => ({ ...prev, [key]: { top, left, label } }));
    // Edge-flip after mount — runs in onMount of the SubMenu, NOT here.
};
```

**The submenu CSS:**

```scss
.menu.sub-menu {
    width: max-content;    // size to content, no JS measurement needed
    min-width: 0;          // already added in #791
}
```

And `<SubMenu>` removes the `visibility: hidden` gate:

```tsx
const subMenu = (
    <div
        ref={...}
        class="menu sub-menu"
        style={{
            top:  `${position()?.top  ?? 0}px`,
            left: `${position()?.left ?? 0}px`,
            position: "absolute",
            "z-index": 1000,
            // visibility: hidden — REMOVED
        }}
    >
```

Edge-flip moves to `onMount`:

```tsx
onMount(() => {
    const el = subMenuRef;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const overflowRight  = r.right  - window.innerWidth;
    const overflowBottom = r.bottom - window.innerHeight;
    if (overflowRight > 0 || overflowBottom > 0) {
        // We're already painted at the primary spot; flip in the NEXT frame.
        // The user might briefly see the unflipped position, but in 99% of
        // viewport configurations the primary spot is correct and no flip
        // is needed — flip-and-flicker is an edge case, not the norm.
        requestAnimationFrame(() => {
            setSubMenuPosition((prev) => ({
                ...prev,
                [parentKey]: {
                    ...prev[parentKey],
                    top:  overflowBottom > 0 ? prev[parentKey].top  - overflowBottom - 10 : prev[parentKey].top,
                    left: overflowRight  > 0 ? itemRect.left - r.width                 : prev[parentKey].left,
                },
            }));
        });
    }
});
```

**Cost dropped:** ~33 ms → < 1 ms.

### 3.2 One persistent overlay, no per-sibling Portal churn

Today, hovering Theme → Opacity:

1. Unmounts the Theme `<SubMenu>` (Portal destroyed, DOM detached, refs nulled)
2. Mounts a new Opacity `<SubMenu>` (Portal created, DOM attached, refs assigned, position computed)

Each round costs ~5 ms of Solid teardown + create overhead, on top of the rendering cost. Even with §3.1 applied, this is wasted work.

**Architecture:**

```
FlyoutMenu
 ├─ <Portal>   (main menu — stays mounted while open)
 │   └─ .menu  (top-level items)
 └─ <Portal>   (single shared submenu overlay — stays mounted while ANY submenu is open)
     └─ .menu.sub-menu
         └─ resolveSubItems(activeSubKey()) | <For>{...}
```

```tsx
function FlyoutMenu(props) {
    const [activeSubPath, setActiveSubPath] = createSignal<MenuItem[] | null>(null);

    return (
        <>
            <Portal>...main menu...</Portal>
            <Portal>
                <Show when={activeSubPath() != null}>
                    <SubMenuOverlay
                        items={activeSubPath()!}
                        position={subPos()}
                    />
                </Show>
            </Portal>
        </>
    );
}
```

When hover moves Theme → Opacity, `activeSubPath` changes from `[themeItem]` to `[opacityItem]`. Solid's `<Show>` keeps the SubMenuOverlay mounted because its boolean stays truthy; only the `items` prop changes, triggering a `<For>` reconciliation inside the overlay. **No Portal mount/unmount.**

For nested submenus (submenu-of-submenu), `activeSubPath` becomes a stack of items: `[themeItem, themeSubcategoryItem]`. The overlay renders **one** submenu per stack level inside the same Portal. Still no remount.

**Cost dropped:** ~5 ms per sibling hop → 0 ms (just a `<For>` reconciliation).

### 3.3 Pre-attached overlay, hidden by `display: none` (the nuclear option)

Even further: instead of `<Show>` mounting/unmounting the overlay, **render it once at FlyoutMenu mount time**, with `display: none`. When a submenu opens, flip to `display: block`. This is what native menus do.

```tsx
const [overlayState, setOverlayState] = createSignal<{
    visible: boolean;
    items: MenuItem[];
    position: { top: number; left: number };
}>({ visible: false, items: [], position: { top: 0, left: 0 } });

// Single Portal, always rendered while FlyoutMenu is open
<Portal>
    <div
        class="menu sub-menu"
        style={{
            display:  overlayState().visible ? "block" : "none",
            position: "absolute",
            top:      `${overlayState().position.top}px`,
            left:     `${overlayState().position.left}px`,
        }}
    >
        <For each={overlayState().items}>...</For>
    </div>
</Portal>
```

The DOM tree is allocated once; subsequent submenu opens just flip `display`. Browser layout/paint is amortized across opens. This is `O(1)` regardless of how many times the user hops between submenus.

Cost: same as §3.2 for the first open; subsequent opens are even cheaper because the overlay element survives.

### 3.4 Replace continuous `autoUpdate` with one-shot + resize/scroll listeners

`autoUpdate` from `@floating-ui/dom` polls position on every animation frame while attached. For the hamburger menu (button doesn't move while open), this is dead-weight.

```ts
// Before:
cleanupAutoUpdate = autoUpdate(referenceEl, floatingEl, updatePosition);

// After:
const reposition = () => updatePosition();
reposition();   // one-shot on open
window.addEventListener("resize", reposition, { passive: true });
window.addEventListener("scroll", reposition, { passive: true, capture: true });
cleanupAutoUpdate = () => {
    window.removeEventListener("resize", reposition);
    window.removeEventListener("scroll", reposition, { capture: true });
};
```

**Cost dropped:** ~0.5 ms per frame of dead work while menu open. Catches the relevant cases (resize, scroll-induced anchor move) without the polling.

### 3.5 Drop the visibility-map state machine, use a stack

Today: `visibleSubMenus: Map<key, {visible: boolean}>` with O(N) ancestor walks per hover. The map exists because the rendering pipeline checks `visible` per item.

With §3.2/§3.3 there's a single overlay; the visibility map collapses to a single `activeSubPath: MenuItem[]` signal. The "ancestor walk" becomes a direct array operation:

```ts
const onParentHover = (item: MenuItem, depth: number) => {
    setActiveSubPath((cur) => {
        const next = cur ? cur.slice(0, depth) : [];
        next.push(item);
        return next;
    });
};
```

No string splits, no O(N) sweep, no ancestor reconstruction. `depth` is known statically per item (it's where in the JSX the item lives) so no walking is needed.

### 3.6 Stop the per-item `isActive` memo invalidation cascade

Today, `hoveredItems` is a single signal; flipping it invalidates every item's `isActive()` memo. For a 10-item menu this is 10 boolean re-evaluations per hover.

After §3.5, item `active` state derives from `activeSubPath`. Solid's fine-grained reactivity will still cascade — but the cost is just `array.includes(item.id)`, no string parsing.

Better: per-item `active` signal local to each item's render scope, set via the hover handler directly. Solid's `<For>` reconciliation already isolates items; the active flag piggybacks on each item's existing render closure. No cascade at all.

---

## 4. Why not just use VSCode's actual menu code?

VSCode's menu lives in `vs/base/browser/ui/contextview` and `vs/base/browser/ui/menu`. Reasons to NOT pull it in:

1. **License + dependencies.** It's MIT but transitively depends on much of VSCode's base layer.
2. **DOM model mismatch.** VSCode's menu is heavily integrated with their action/command system; would need significant adapter code.
3. **Solid mismatch.** It's vanilla DOM; we'd need a Solid wrapper.

We're already 90% of the way to native parity with the changes in §3. Pulling in a third-party menu library isn't justified.

---

## 5. Could we go fully CSS-only?

For a pure-display submenu (no scroll, no virtualization), yes: `details/summary` + `:hover` pseudo-selectors give you submenu open-on-hover with zero JS. **But** AgentMux menus need keyboard navigation, radio state, click-outside dismissal, accessibility (`aria-expanded`, `aria-haspopup`, `role="menu"`) — all of which require JS state. Pure CSS gets us the *opening* for free; the *interaction* still needs the state machine.

A hybrid: open is CSS-controlled (no JS hover handler at all — pure `:hover` selector on the parent item); state syncs happen on **click**, not hover. This gets us:

- **Zero hover latency** by definition — the browser shows the submenu on hover via CSS, no script involved.
- Keyboard navigation still works via the JS state machine.
- Radio `checked` state still routes through `SetConfigCommand` on click.

Worth prototyping as Phase 6 after §3.1–3.5 land.

---

## 6. Benchmarking plan

Before/after measurement. Run all in `task dev` against the hamburger menu, with the perf probe enabled (Ctrl+Shift+D → Agent pane perf → Menu open timing).

| Metric | Today | Target | How to measure |
|---|---|---|---|
| `pointerenter` → submenu first paint | ~33 ms | < 16 ms | `PerformanceObserver({ entryTypes: ["event"] })` on the menu container, look at `processingStart - startTime` and the next `paint` event |
| Theme → Opacity hop latency | ~10 ms (mount cost) | < 1 ms | Same, on consecutive hover events |
| `forced reflow` count per menu open | 2 (one from `setTimeout`, one from `autoUpdate`) | 0 | Chrome DevTools Performance panel; look for "Recalculate Style" + "Layout" pairs |
| Continuous CPU while menu idle | ~0.5 ms / frame from `autoUpdate` | 0 | `chrome://tracing` |

Instrument once, run the same scenario against `main` and against the snappy branch, post the table to the PR.

---

## 7. Phased ship order (revised)

This supersedes the Phase 1–5 plan in `flyout-menu-hover-delay-2026-05-10.md`.

| Phase | What | Effort | Risk | Latency win |
|---|---|---|---|---|
| **A** | §3.1 — synchronous primary position, drop `setTimeout(0)` and `visibility: hidden`. Edge-flip on `onMount` only when needed. | 0.5 day | Low (FlyoutMenu only, no API change) | **Biggest** — 33 ms → ~2 ms |
| **B** | §3.4 — one-shot `updatePosition` instead of `autoUpdate` | 0.25 day | Low | Continuous CPU recovered |
| **C** | §3.2 — single shared SubMenuOverlay; activeSubPath stack | 1 day | Medium (FlyoutMenu internals reshape; same external API) | Sibling hops 10 ms → ~0 ms |
| **D** | §3.5 + §3.6 — drop visibility-map, drop ancestor walk, drop per-item memo cascade | 0.5 day | Low; bundled with C | Cleanup + microbenchmark |
| **E** | §3.3 — pre-attached overlay with `display: none` toggle | 0.5 day | Low (additive on top of C) | First-open allocation amortized |
| **F** (optional) | §5 — CSS-only `:hover` open with JS keyboard/click handlers | 1 day | Medium (accessibility audit needed) | Zero JS in the hot path |
| **G** (optional) | Intent debounce against menu-chase (50ms) | 0.25 day | Low | Polish; only matters after A–F land |

**Recommendation:** ship A + B as one PR (small, surgical, immediate win). Then C + D + E as a second PR (architectural refactor, biggest internal cleanup). Skip F unless we find a real reason; G as final polish.

---

## 8. Out of scope

- **Context-menu (right-click) snappiness.** Different code path (`ContextMenuModel`), worth a separate review if we observe similar lag.
- **Toast/tooltip snappiness.** Tooltip already uses `requestAnimationFrame`; if there's a reported delay there, treat as a separate ticket.
- **Replacing FlyoutMenu with `@kobalte/core` or another Solid menu library.** Would deliver native-feel by default, but is a big migration. Worth a separate spec if we go that direction.

---

## 9. Codebase rule (proposed)

Add to `CLAUDE.md` (project section):

> **No `setTimeout(_, 0)`.** It clamps to ≥ 4 ms and runs as a macrotask, typically after the next paint. If you need to defer until "after the DOM is attached", use `onMount` (Solid) or `queueMicrotask`. If you need "after the next paint", use `requestAnimationFrame`. If you need "when the browser is idle", use `requestIdleCallback`. `setTimeout(_, 0)` is always wrong.

The handful of existing sites (§1.1) should be migrated as part of this work or as a single follow-up sweep.

---

## 10. Open questions

- **What's the real keyboard-navigation contract for FlyoutMenu?** None of the current callers use keyboard nav (hamburger menu, chat agent picker). If keyboard nav becomes a requirement, the state machine in §3.5 needs an active-index in addition to active-path. Worth confirming before C lands.
- **Do we ever need a submenu wider than `max-content`?** Today no — all submenu items fit. If we ever add inline forms or descriptions, `width: max-content` will need to become `width: clamp(min, max-content, viewport-bound)` and §3.1's "no measurement" claim weakens. Watch for it.
- **Should we publish a `<Menu>` primitive at the Solid level (Kobalte-style)?** Out of scope for this work, but the architecture in §3 essentially is one — worth thinking about packaging if more flyout-menu-like UIs land.

---

## 11. Cross-references

- Predecessor analysis: `docs/analysis/flyout-menu-hover-delay-2026-05-10.md`
- Implementation site: `frontend/app/element/flyoutmenu.tsx`, `flyoutmenu.scss`
- Hamburger caller: `frontend/app/tab/tabbar.tsx`
- Other submenu caller: `frontend/app/view/chat/data.tsx`
- Project specs index: `docs/specs/`
- Floating UI docs: https://floating-ui.com/docs/autoUpdate, https://floating-ui.com/docs/computePosition
- Web Animation timing primer: https://developer.mozilla.org/en-US/docs/Web/API/HTML_DOM_API/Microtask_guide
