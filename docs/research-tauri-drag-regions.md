# Tauri v2 Window Drag Regions — Definitive Research

**Date:** 2026-03-18
**Status:** Actionable findings — switch to `startDragging()` API

---

## Key Finding: `data-tauri-drag-region` Does NOT Cascade to Children

Tauri injects `drag.js` into every webview. On `mousedown`, it walks the `composedPath()` from `e.target` upward:

1. **Bare attribute** (`data-tauri-drag-region`): Only triggers drag if the click was **directly on that element** (`el === composedPath[0]`). Children do NOT inherit drag.
2. **`="false"`**: Explicitly opts out. Works in v2.5+.
3. **`="deep"`**: Entire subtree is a drag zone (clickable elements auto-excluded). **Not yet released** — merged to `dev` branch March 10, 2026 (PR #15062), pending next stable after v2.10.3.

**This is why our approach of setting `data-tauri-drag-region="true"` on container divs didn't work** — only direct clicks on the container itself (not on child spans, divs, or buttons) triggered drag. Empty space between children belongs to the container, but text/elements inside don't.

## Why `-webkit-app-region: drag` Was Reverted

Tauri tried this in beta (PR #9789) and **reverted it** (PR #9860):
- Treats entire area as native titlebar — ALL clicks swallowed by OS
- Right-click always shows system context menu
- Only works on Chromium-based WebView2 (Windows)

**Do not use CSS app-region for drag.**

---

## The Solution: Programmatic `startDragging()`

```tsx
import { getCurrentWindow } from "@tauri-apps/api/window";

const appWindow = getCurrentWindow();

const handleMouseDown = (e: MouseEvent) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    // Don't drag from interactive elements
    if (target.closest("button, input, select, a, [data-no-drag]")) return;

    if (e.detail === 2) {
        appWindow.toggleMaximize();
    } else {
        e.preventDefault();
        appWindow.startDragging();
    }
};

<div class="titlebar" onMouseDown={handleMouseDown}>
    <span>App Title</span>
    <button>Close</button>  {/* excluded by closest() check */}
</div>
```

**Required permission:**
```json
"permissions": ["core:window:allow-start-dragging"]
```

### Advantages

- **Full control** over which elements trigger drag vs. receive clicks
- **No attribute inheritance confusion** — you write the exclusion logic
- **Works identically** across Windows, macOS, Linux
- **Double-click-to-maximize** trivially handled via `e.detail === 2`
- **No conflict** with widget clicks, tab drag, pane drag, or any other interaction

### Why This Is Better Than Attributes

| Problem | Attribute approach | startDragging() |
|---------|-------------------|-----------------|
| Child elements don't inherit | Must add attribute to every leaf | One handler on container |
| Buttons swallowed | `="false"` unreliable pre-v2.5 | `closest()` check excludes them |
| Gaps between elements | Only container gets drag, not gaps | Container mousedown catches gaps |
| Conflict with DnD | Tauri intercepts before HTML5 DnD | You control when to call startDragging() |
| Platform differences | Injected JS behaves differently | API is cross-platform |

---

## Implementation Plan for AgentMux

### Step 1: Replace all `data-tauri-drag-region` with `startDragging()`

**window-header.tsx:**
```tsx
import { getCurrentWindow } from "@tauri-apps/api/window";

const WindowHeader = (props) => {
    const appWindow = getCurrentWindow();

    const handleMouseDown = (e: MouseEvent) => {
        if (e.button !== 0) return;
        const target = e.target as HTMLElement;
        // Don't drag from interactive elements
        if (target.closest("button, input, select, a, .tab, .action-widget-slot, [data-no-drag]")) return;

        if (e.detail === 2) {
            appWindow.toggleMaximize();
        } else {
            e.preventDefault();
            appWindow.startDragging();
        }
    };

    return (
        <div class="window-header" onMouseDown={handleMouseDown}>
            <TabBar ... />
            <SystemStatus />
        </div>
    );
};
```

### Step 2: Remove all `data-tauri-drag-region` attributes

- `window-header.tsx`: remove `{...dragProps}`
- `tabbar.tsx`: remove `data-tauri-drag-region="false"`
- `system-status.tsx`: remove `data-tauri-drag-region="false"`
- `action-widgets.tsx`: remove all `data-tauri-drag-region` attrs
- `tab.tsx`: remove `data-tauri-drag-region="false"`

### Step 3: Remove `useWindowDrag` hook

No longer needed — the hook was just `{ "data-tauri-drag-region": true }`.

### Step 4: Remove WindowDrag spacer elements

No longer needed — the entire header is the drag zone, with `closest()` excluding interactive elements.

### Step 5: Add permission

In `src-tauri/capabilities/`:
```json
"permissions": ["core:window:allow-start-dragging"]
```

### Step 6: Linux consideration

The current code skips drag attributes on Linux (`isLinux() ? {} : ...`) because Linux uses a GTK native drag handler (`drag.rs`). With `startDragging()`, we may be able to unify — Tauri's `startDragging()` should work on Linux too via the GTK drag API. Test this.

---

## Future: `data-tauri-drag-region="deep"`

When Tauri releases a stable version with PR #15062, the `="deep"` attribute becomes viable:
- Set `data-tauri-drag-region="deep"` on the window header
- All children become drag zones except `button`, `input`, `select`, `a`, `textarea`, `label`, `summary`, `contenteditable`, `tabindex != -1`
- Custom interactive `<div>` elements need `data-tauri-drag-region="false"` to opt out

This would be simpler than `startDragging()` but requires a Tauri version that hasn't been released yet.

---

## References

- [Tauri drag.js source](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri/src/window/scripts/drag.js)
- [PR #15062: `="deep"` mode](https://github.com/tauri-apps/tauri/pull/15062)
- [PR #13269: `="false"` support](https://github.com/tauri-apps/tauri/pull/13269)
- [PR #9860: Revert CSS app-region](https://github.com/tauri-apps/tauri/pull/9860)
- [Issue #9901: Child elements can't trigger events](https://github.com/tauri-apps/tauri/issues/9901)
- [Window Customization docs](https://v2.tauri.app/learn/window-customization/)
