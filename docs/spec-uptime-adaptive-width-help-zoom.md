# Spec: Adaptive Uptime Width + Help View Zoom

**Date:** 2026-03-22
**Status:** Proposed

---

## Feature 1 — Adaptive Uptime Min-Width

### Problem
`.stat-uptime` currently uses a fixed `min-width`. As uptime grows from minutes → hours → days, the
field expands mid-render, pushing CPU/mem/disk stats to the right on every tick. A single large
min-width (e.g. `12ch`) solves the jitter but wastes horizontal space for 99% of sessions.

### Solution: Tier-based inline `min-width`
Replace the CSS `min-width` with a reactive inline style computed from the current uptime.
The width only ever steps _up_, never down, and only at the natural format transition points.

#### Format tiers

| Uptime range | Format example | Max string | `min-width` |
|---|---|---|---|
| 0 – 59:59 | `5:03` | `59:59` | `5ch` |
| 1:00:00 – 23:59:59 | `1:03:45` | `23:59:59` | `8ch` |
| 1:00:00:00 + | `1:02:03:04` | `365:23:59:59` | `12ch` |

Only **3 possible expansions** ever. After the first 24-hour mark the field never grows again.

#### Implementation

In `BackendStatus.tsx`, add a derived signal:

```ts
const uptimeMinWidth = (): string => {
    const s = uptimeSecs();
    if (s < 3600)  return "5ch";   // M:SS
    if (s < 86400) return "8ch";   // H:MM:SS
    return "12ch";                  // D:HH:MM:SS
};
```

Apply to the span:

```tsx
<span class="stat-mono stat-uptime" style={{ "min-width": uptimeMinWidth() }}>
    {formatUptime(uptimeSecs())}
</span>
```

Remove `min-width` from `.stat-uptime` in `StatusBar.scss` (the inline style takes over).

#### Notes
- The `ch` unit is based on the `0` glyph width of the current font — appropriate for the
  monospace `.stat-mono` class.
- `uptimeMinWidth()` is pure (no side effects), cheaply recomputed each second.
- No persistence needed — the field grows exactly once per tier and stays there.

---

## Feature 2 — Help View Zoom (Ctrl+/- and Ctrl+Wheel)

### Problem
The help/QuickTips pane has no zoom. Users who zoom terminal panes expect the same gesture
to work in the help pane.

### Solution: CSS `font-size` scaling with reactive signal

Use a `createSignal<number>` for zoom factor (0.5–2.0, default 1.0) stored on the
`HelpViewModel`. Apply as a CSS `font-size` percentage to the content wrapper. Because the
content is regular HTML/markdown, scaling `font-size` causes all child text, headings, and
spacing to scale proportionally — fully responsive with no layout breakage.

**Do not use CSS `transform: scale()`** — it scales the element without reflowing, causing
overflow. **Do not use CSS `zoom`** — non-standard and inconsistent across engines.

#### Zoom range & steps
Mirrors terminal zoom:
- Range: 50% – 200% (`MIN_ZOOM = 0.5`, `MAX_ZOOM = 2.0`)
- Keyboard step: 10% (`KEYBOARD_STEP = 0.1`)
- Wheel step: 5% (`WHEEL_STEP = 0.05`)
- Reset: Ctrl+0 → 100%

#### Persistence
Store zoom in block meta as `"help:zoom"` (same pattern as `"term:zoom"`). This way the user's
preferred size survives pane close/reopen.

#### Keyboard events
The content div needs `tabIndex={0}` (or capture from the block frame). Add `onKeyDown` handler:

```ts
const handleKeyDown = (e: KeyboardEvent) => {
    if (!e.ctrlKey && !e.metaKey) return;
    if (e.key === "=" || e.key === "+") { e.preventDefault(); adjustZoom(KEYBOARD_STEP); }
    if (e.key === "-")                  { e.preventDefault(); adjustZoom(-KEYBOARD_STEP); }
    if (e.key === "0")                  { e.preventDefault(); resetZoom(); }
};
```

#### Wheel events
Add `onWheel` on the content wrapper:

```ts
const handleWheel = (e: WheelEvent) => {
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();
    adjustZoom(e.deltaY > 0 ? -WHEEL_STEP : WHEEL_STEP);
};
```

Use `{ passive: false }` if attaching via `addEventListener` directly (needed for `preventDefault`).
In SolidJS JSX, `onWheel` already works non-passively.

#### Zoom indicator
Call `showZoomIndicator` from `zoom.platform` to display the transient overlay, same as terminal.

#### Apply zoom to DOM

```tsx
<div
    class="help-view-content"
    style={{ "font-size": `${zoomFactor() * 100}%` }}
    tabIndex={0}
    onKeyDown={handleKeyDown}
    onWheel={handleWheel}
>
    <QuickTips />
</div>
```

#### Responsiveness
Since `font-size` scaling causes reflow:
- All `em`/`rem`-based spacing in QuickTips scales automatically.
- Fixed `px` values in QuickTips SCSS should be converted to `em` where they affect visual rhythm
  (padding, gap, heading margins). Pixel borders are fine to leave as-is.
- The outer wrapper keeps `overflow-auto` so content that grows beyond the pane still scrolls.

#### HelpViewModel changes
```ts
class HelpViewModel implements ViewModel {
    zoomAtom: SignalAtom<number>;

    constructor(blockId: string) {
        this.blockId = blockId;
        this.zoomAtom = useBlockAtom(blockId, "helppzoomatom", () =>
            createSignal<number>(/* read help:zoom from block meta or default */ 1.0)
        );
    }

    adjustZoom(step: number): void { /* clamp + setMeta("help:zoom") */ }
    resetZoom(): void              { /* setMeta("help:zoom", null) */ }
}
```

---

## Files to change

### Feature 1 (uptime adaptive width)
| File | Change |
|---|---|
| `frontend/app/statusbar/BackendStatus.tsx` | Add `uptimeMinWidth()` derived signal; add `style` to uptime span |
| `frontend/app/statusbar/StatusBar.scss` | Remove `min-width` from `.stat-uptime` |

### Feature 2 (help zoom)
| File | Change |
|---|---|
| `frontend/app/view/helpview/helpview.tsx` | Add zoom signal, keydown/wheel handlers, `font-size` style |
| `frontend/app/view/helpview/helpview.scss` | New file: convert fixed-px values in QuickTips wrapper to `em` |
| `frontend/app/element/quicktips.tsx` | Audit for fixed-px spacing that would break zoom |
| `frontend/types/gotypes.d.ts` | _(no change needed — `help:zoom` goes through existing SetMeta RPC)_ |

---

## Open questions
1. Should help zoom default to the same value as the user's last terminal zoom, or always 100%?
   → Recommend 100% (independent preference).
2. Should Ctrl+Wheel on the help pane also affect chrome zoom (title bar), or only content?
   → Content only. Chrome zoom is scoped to hovering over the title/status bar in `app.tsx`.
