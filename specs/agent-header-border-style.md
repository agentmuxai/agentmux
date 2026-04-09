# Agent Pane Header: Border Style Instead of Solid Background

## Summary

Change agent-loaded terminal pane headers from a solid colored background to a
transparent/black background with colored top and bottom borders. The header's
top border and bottom border use the agent's color, creating two horizontal
colored lines that frame the header content. This matches the pane's outer
border color for visual consistency.

## Motivation

1. **Diagnostic value:** The Win11 focus border bug renders `var(--accent-color)`
   as black on `backdrop-filter` elements, but agent inline HEX colors work.
   Multiple fix attempts (stacking context, HEX conversion, backdrop-filter
   removal) were tried but never merged (`agentx/win11-focus-border-fix`,
   commits `330334f6`, `398653b2`, `724fa2a9`). By making the header use the
   same color path as the border (inline HEX on the same element tree), we
   create a consistent visual language AND gain another data point: if the
   header borders render correctly but the pane border doesn't, we can isolate
   the bug to `backdrop-filter` compositing specifically.

2. **Visual refinement:** Solid colored headers are heavy. Two thin colored
   lines framing the header text are more subtle and let the dark theme
   breathe. The agent identity is still clearly communicated through color.

3. **Consistency:** The pane border and header borders now use the same color,
   creating a unified visual frame around the pane.

## Current State

Agent-loaded terminal headers (`blockframe.tsx:283-290`):
```tsx
const headerStyle = createMemo<JSX.CSSProperties>(() => {
    const style: JSX.CSSProperties = {};
    const ac = agentColor();         // e.g. "#ef4444"
    const atc = agentTextColor();    // e.g. "#ffffff"
    if (ac) style["background-color"] = ac;   // Solid fill
    if (atc) style.color = atc;
    return style;
});
```

Applied to `.block-frame-default-header` at line 300:
```tsx
<div class="block-frame-default-header" style={headerStyle()}>
```

Default header CSS (`block.scss:82-92`):
```scss
.block-frame-default-header {
    max-height: var(--header-height);
    min-height: var(--header-height);
    display: flex;
    padding: 4px 5px 4px 10px;
    align-items: center;
    gap: 8px;
    font: var(--header-font);
    zoom: var(--zoomfactor, 1);
    border-bottom: 1px solid var(--border-color);
    border-radius: var(--block-border-radius) var(--block-border-radius) 0 0;
}
```

## Proposed Change

### Visual (ASCII mockup)

**Before (solid background):**
```
+--[ red fill ]==================================+
|  > AgentX                          [_][M][X]   |  <- solid red bg, white text
+================================================+
|                                                 |
|  terminal content                               |
|                                                 |
+-------------------------------------------------+
   ^-- red border (agent color, works on Win11)
```

**After (border lines):**
```
+--[ red top border ]============================+
|  > AgentX                          [_][M][X]   |  <- black/transparent bg
+--[ red bottom border ]=========================+
|                                                 |
|  terminal content                               |
|                                                 |
+-------------------------------------------------+
   ^-- red border (agent color, works on Win11)
```

### Code Changes

**`blockframe.tsx` -- headerStyle memo (~line 283):**

```tsx
const headerStyle = createMemo<JSX.CSSProperties>(() => {
    const style: JSX.CSSProperties = {};
    const ac = agentColor();
    const atc = agentTextColor();
    if (ac) {
        // Border lines instead of solid fill
        style["border-top"] = `2px solid ${ac}`;
        style["border-bottom-color"] = ac;
        // Agent text color on dark background (slightly dimmed for subtlety)
        style.color = atc ?? ac;
    }
    return style;
});
```

**No SCSS changes needed.** The header already has `border-bottom: 1px solid var(--border-color)`.
The inline style overrides `border-bottom-color` to the agent color when an agent is present.
We add a `border-top` inline style for the top colored line.

### What stays the same

- Non-agent terminals: no change (no agent color detected, header stays default)
- Pane outer border: still uses `--block-agent-color` via `.has-agent-color .block-mask`
- Agent detection logic: unchanged
- `--block-agent-color` CSS variable: still set on `.block` for the outer border

## Affected Files

| File | Change |
|------|--------|
| `frontend/app/block/blockframe.tsx` | Modify `headerStyle` memo (~line 283-290) |

One file, ~5 lines changed.

## Edge Cases

1. **Header border-top adds 2px height.** The header has `max-height: var(--header-height)`.
   The `border-top` is inside the box model (default `box-sizing: content-box`), but the
   header uses flex layout with `min-height`/`max-height`. Verify the header doesn't clip
   or shift content. May need to adjust padding-top from `4px` to `2px` to compensate.

2. **Agent text color on dark bg.** Currently agent text color is white-on-colored-bg.
   On dark bg, white text still works. But some agents have dark colors (e.g. AgentA
   `#1e3a5f` dark blue) where the border lines may be hard to see. This is fine -- the
   same color is used for the pane border and is already visible there.

3. **Preview mode.** Preview panes use `block-preview` class with 70% opacity background.
   Agent headers in preview should inherit the same border treatment. No special handling
   needed since `headerStyle()` is applied unconditionally.

## Testing

- [ ] Agent terminal (e.g. AgentX): header shows red top/bottom borders, dark bg, colored text
- [ ] Non-agent terminal: header unchanged (default dark bg, dim border-bottom)
- [ ] Multiple agents: each shows its own color borders (AgentX red, Agent1 green, etc.)
- [ ] Header controls (minimize, magnify, close) still visible and clickable
- [ ] Header text not clipped by added border-top
- [ ] Pane outer border still matches header border color
- [ ] Win11: verify both header borders and pane border render correctly
