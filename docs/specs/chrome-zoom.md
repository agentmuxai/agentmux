# Chrome Zoom: Status Bar + Title Bar

## Status: Spec

## Summary

The window chrome (top title bar + bottom status bar) should be zoomable as a single unit. Ctrl+Scroll over either bar zooms both together. This is independent of per-pane terminal zoom.

## Motivation

When terminal panes are zoomed up, the title bar and status bar stay at fixed size, creating a visual mismatch. Users should be able to scale the chrome to match.

## Desired Behavior

| Input | Target |
|-------|--------|
| Ctrl+Scroll over title bar | Zoom title bar + status bar together |
| Ctrl+Scroll over status bar | Zoom title bar + status bar together |
| Ctrl+Scroll over terminal pane | Zoom that terminal pane (existing) |
| Ctrl+/- | Zoom focused terminal pane (existing, unchanged) |

- Chrome zoom is a single global value (not per-pane).
- Persisted in settings as `ui:chromezoom` (float, default 1.0).
- Range: 0.5x to 2.0x, same as terminal zoom.
- Does NOT affect terminal pane content.

## Implementation

### 1. New atom: `chromeZoomAtom`

In `frontend/app/store/zoom.ts`:

```ts
export const chromeZoomAtom = atom<number>(1.0);
```

On startup, read from settings (`ui:chromezoom`). On change, persist back.

### 2. Apply zoom via CSS custom property

Set `--chrome-zoom` on the root element whenever `chromeZoomAtom` changes:

```ts
// In a useEffect or subscription
document.documentElement.style.setProperty("--chrome-zoom", String(zoom));
```

Then in SCSS for both bars:

```scss
.window-header {
    font-size: calc(var(--chrome-zoom, 1) * 13px);
    height: calc(var(--chrome-zoom, 1) * 40px);
    // icon sizes, padding, etc. scale similarly
}

.statusbar {
    font-size: calc(var(--chrome-zoom, 1) * 12px);
    height: calc(var(--chrome-zoom, 1) * 28px);
}
```

Alternatively, use `transform: scale(var(--chrome-zoom))` on the bar containers with `transform-origin: center` — simpler but may cause sub-pixel issues.

### 3. Update wheel handler in `app.tsx`

The existing `AppZoomHandler` already resolves `data-blockid` from the event target. For chrome zoom, check if the wheel event target is inside `.window-header` or `.statusbar`:

```ts
const handleWheel = (e: WheelEvent) => {
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();

    const target = e.target as HTMLElement;

    // Check if hovering over chrome (title bar or status bar)
    if (target.closest(".window-header") || target.closest(".statusbar")) {
        if (e.deltaY > 0) chromeZoomOut();
        else if (e.deltaY < 0) chromeZoomIn();
        return;
    }

    // Otherwise, zoom the terminal pane under cursor (existing logic)
    const blockEl = target.closest("[data-blockid]");
    const blockId = blockEl?.getAttribute("data-blockid");
    if (!blockId) return;

    if (e.deltaY > 0) zoomBlockOut(blockId, WHEEL_STEP);
    else if (e.deltaY < 0) zoomBlockIn(blockId, WHEEL_STEP);
};
```

### 4. Persist chrome zoom

```ts
export function chromeZoomIn(step: number = WHEEL_STEP): void {
    const current = globalStore.get(chromeZoomAtom);
    setChromeZoom(current + step);
}

export function chromeZoomOut(step: number = WHEEL_STEP): void {
    const current = globalStore.get(chromeZoomAtom);
    setChromeZoom(current - step);
}

function setChromeZoom(factor: number): void {
    const clamped = clampZoom(roundZoom(factor));
    globalStore.set(chromeZoomAtom, clamped);
    document.documentElement.style.setProperty("--chrome-zoom", String(clamped));
    // Persist to settings
    fireAndForget(() =>
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("client", clientId),
            meta: { "ui:chromezoom": clamped === 1.0 ? null : clamped },
        })
    );
    showZoomIndicator(`Chrome ${Math.round(clamped * 100)}%`);
}
```

### 5. Files to modify

| File | Change |
|------|--------|
| `frontend/app/store/zoom.ts` | Add `chromeZoomAtom`, `chromeZoomIn`, `chromeZoomOut`, `setChromeZoom` |
| `frontend/app/app.tsx` | Update wheel handler to detect chrome hover |
| `frontend/app/window/window-header.scss` | Use `var(--chrome-zoom)` for sizing |
| `frontend/app/statusbar/StatusBar.scss` | Use `var(--chrome-zoom)` for sizing |
| `frontend/wave.ts` | Load persisted chrome zoom on startup |

### 6. Edge cases

- **Chrome zoom + pane zoom independent:** Zooming chrome doesn't affect terminals and vice versa.
- **Tab bar is part of chrome:** The tab bar sits inside the header area — it scales with title bar zoom.
- **Zoom indicator:** Shows "Chrome 120%" to distinguish from pane zoom indicators.
- **Reset:** Ctrl+0 only resets focused pane zoom. Chrome zoom reset could be a right-click menu option on the status bar, or a separate keybinding (TBD).
