# SPEC: Always-visible window drag handle (grip) in the tab bar

**Status:** Draft
**Date:** 2026-06-06
**Author:** AgentA

---

## 1. Problem

On Windows/Linux the entire tab bar is technically a drag region, but the only
reliably empty draggable space is `tab-bar-fill` — the slack area to the right
of the last tab. When the tab bar is full, `tab-bar-fill` shrinks to zero and
there is nowhere to grab the window without closing or moving a tab.

The hamburger (`<HamburgerMenu>`) sits at the far left but its element has
`data-drag-region="false"` (it's a button). The window is effectively
un-draggable with a full tab strip.

---

## 2. Solution: fixed-width grip handle between hamburger and tabs

Insert a `<WindowDragHandle>` component immediately after `<HamburgerMenu>` in
the tab bar, to the LEFT of the tab scroll container. The handle:

- is **always present** and always the same size regardless of tab count
- shows a **grip icon** (⠿ — see §3) as a visual affordance
- carries `data-drag-region="true"` so the existing win32/linux drag hook picks
  it up with zero extra logic
- is **non-interactive** beyond drag: no click action, no menu, no tooltip

On macOS the handle is **not rendered** — the traffic light area + the native
window header already covers this, and the macOS drag hook uses a different
mechanism (WebView-level `data-drag-region` → OS HTCAPTION).

---

## 3. Grip icon

Unicode **⠿** (U+283F, Braille pattern dots-1-2-3-4-5-6) renders as a tight
2×3 dot grid and is the conventional "grip" glyph in monospace/icon contexts.
If the chosen font renders it too small or off-center, fall back to a small
inline SVG (two columns of three dots, 2px diameter, 3px vertical gap, 5px
column gap).

The icon color: `var(--color-secondary-text)` at 50% opacity (subdued but
readable), brightening to 70% on hover over the handle.

---

## 4. Size and hit target

| Property | Value |
|---|---|
| Width | 20px |
| Height | 100% of `.tab-bar` (inherits flex align-stretch) |
| Min hit target | 20 × 28px (meets WCAG 2.5.8 minimum) |
| Cursor | `grab` (→ `grabbing` while held, via CSS `:active`) |
| Padding | 0 4px (icon centered) |

The handle sits between the hamburger button and the `.tab-bar-scroll`
container. It does not push tabs; the flex layout of `.tab-bar` absorbs it as a
fixed-width flex item.

---

## 5. Technical implementation

### 5.1 Component

New file: `frontend/app/window/WindowDragHandle.tsx`

```tsx
// Renders only on Windows and Linux; returns null on macOS.
// data-drag-region="true" hooks into the existing useWindowDrag hook
// (win32: native move loop via start_window_drag IPC; linux: JS-driven).
export function WindowDragHandle(): JSX.Element {
    if (isMacOS()) return null;
    return (
        <div class="window-drag-handle" data-drag-region="true" aria-hidden="true">
            ⠿
        </div>
    );
}
```

### 5.2 Tab bar insertion

`frontend/app/tab/tabbar.tsx` — inside the `<Show when={!isMacOS()}>` block,
after `<HamburgerMenu />`:

```tsx
<Show when={!isMacOS()}>
    <HamburgerMenu />
    <WindowDragHandle />  {/* ← new */}
</Show>
```

### 5.3 CSS

```css
.window-drag-handle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    flex-shrink: 0;
    cursor: grab;
    color: var(--color-secondary-text);
    opacity: 0.5;
    font-size: 14px;
    user-select: none;
}

.window-drag-handle:hover  { opacity: 0.7; }
.window-drag-handle:active { cursor: grabbing; }
```

### 5.4 Drag machinery — no changes needed

`useWindowDrag.win32.ts`: `isInDragRegion()` walks up the DOM looking for
`data-drag-region`. The handle's own attribute (`"true"`) is found immediately —
no changes to the hook. The native move loop fires exactly as it does for
`tab-bar-fill`.

`useWindowDrag.linux.ts`: same attribute-walk logic, same result.

---

## 6. Platform matrix

| Platform | Rendered | Drag mechanism |
|---|---|---|
| Windows | Yes | `data-drag-region` → `start_window_drag` IPC → native move loop (SetCapture) |
| Linux | Yes | `data-drag-region` → JS-driven `set_window_position` per-move |
| macOS | No | Traffic lights + macOS WebView drag handle the window |

---

## 7. Open questions / out of scope

| # | Question | Default |
|---|---|---|
| 1 | Show on floater windows too? | Yes — same component, same region; floaters already have their own drag header but consistency doesn't hurt |
| 2 | Reduce opacity when maximized (dragging a maximized window unmaximizes it)? | No change for now; existing behavior on `tab-bar-fill` is the same |
| 3 | RTL layout — does the handle move to the right of the hamburger (now on the right)? | Out of scope; RTL not currently supported |

---

## 8. Key files

| File | Change |
|---|---|
| `frontend/app/window/WindowDragHandle.tsx` | New component |
| `frontend/app/tab/tabbar.tsx` | Insert `<WindowDragHandle />` after `<HamburgerMenu />` |
| `frontend/app/styles/tabbar.css` (or equivalent) | `.window-drag-handle` styles |
