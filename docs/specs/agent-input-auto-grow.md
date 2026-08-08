# Agent Input Auto-Grow Textarea

**Status:** Proposed (implemented — see note below)
**Date:** 2026-04-09

> **2026-08-07 audit note:** Implemented, differently than proposed here —
> shipped via plain CSS `field-sizing: content` rather than the JS resize
> handler this doc describes. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.

## Summary

The agent pane input textarea should start at 1 line and automatically
grow vertically as the user types multi-line content. The manual resize
handle (bottom-right drag) should be removed — height is driven purely
by content.

## Current Behavior

- Fixed at `rows={2}` (always 2 lines tall regardless of content)
- `resize: vertical` CSS allows manual drag-to-resize
- `min-height: 28px`

## Desired Behavior

- Start at 1 line tall (single row)
- Grow vertically as the user adds newlines or wraps long lines
- Shrink back down when text is deleted
- Cap at a max height (e.g., 200px / ~10 lines) to avoid consuming the
  whole pane — overflow scrolls within the textarea after that
- Reset to 1 line after sending a message
- No manual resize handle

## Implementation

### AgentFooter.tsx

1. Change `rows={2}` → `rows={1}`
2. Add an `autoGrow` function that sets `textarea.style.height` based
   on `scrollHeight` after each input event
3. Reset height on send (message cleared → textarea shrinks)
4. Use a ref to access the textarea DOM element

```typescript
const autoGrow = (el: HTMLTextAreaElement) => {
    el.style.height = "auto";            // reset to measure
    el.style.height = el.scrollHeight + "px";  // set to content
};
```

Call `autoGrow` in `onInput` after updating the signal, and after
`setMessage("")` in `handleSend`.

### agent-view.scss

1. Change `resize: vertical` → `resize: none`
2. Add `max-height: 200px` and `overflow-y: auto`
3. Keep `min-height` at one line (~20px with current font-size)

## Files to Change

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/AgentFooter.tsx` | Auto-grow logic, rows=1, ref |
| `frontend/app/view/agent/agent-view.scss` | `resize: none`, `max-height`, adjust `min-height` |

## Testing

1. Open agent pane → textarea shows as a single line
2. Type a long message → wraps, textarea grows
3. Press Shift+Enter → new line, textarea grows
4. Delete lines → textarea shrinks
5. Send message → textarea resets to 1 line
6. Paste a 20-line block → grows to max-height, scrollbar appears
7. No resize handle visible in bottom-right corner
