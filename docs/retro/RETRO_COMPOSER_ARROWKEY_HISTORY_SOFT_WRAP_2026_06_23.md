# Retro: Composer Arrow-Key History Navigation Breaks on Soft-Wrapped Input

**Date:** 2026-06-23  
**Component:** `AgentFooter` — sent-message history recall (ArrowUp / ArrowDown)  
**Severity:** UX regression — medium (affects any multi-line message typed without explicit newlines)  
**Status:** Fixed

---

## What Happened

When a user typed a long message that visually wrapped to 2+ lines in the agent
pane composer (without pressing Enter/Shift+Enter — i.e. no `\n` in the text),
pressing ArrowUp at any visual position would immediately replace the composer
content with the previous sent message.

Expected: ArrowUp moves the cursor up one visual line within the current text;
only when the cursor is already on the first visual line does it navigate to the
previous sent message.

---

## Root Cause

`AgentFooter.tsx` computed "cursor is on first line" as:

```typescript
const caretOnFirstLine = !textareaRef.value.slice(0, textareaRef.selectionStart).includes("\n");
```

This checks whether there is a physical newline character (`\n`) anywhere before
the cursor. For soft-wrapped text — a single paragraph that the browser word-wraps
visually into multiple rows — there are no `\n` characters, so the check always
returns `true`. The textarea thought the cursor was permanently on "the first
line" regardless of where the user had navigated within a visually multi-line
message.

The same bug existed in the symmetric `caretOnLastLine` check, meaning ArrowDown
on a soft-wrapped message that had no trailing `\n` would also immediately jump
forward to the next history entry instead of moving down one visual row.

### Why the original code appeared to work

For the most common case — a user pressing ArrowUp on an **empty** composer, or
on a single-line message — the check is correct: no `\n` before the cursor, Y
position is 0, cursor is on the first line. The bug only surfaces when the
composer has a paragraph long enough to soft-wrap.

---

## The Fix

Replaced the `\n` scan with a visual-line measurement using a temporary mirror
div that replicates the textarea's CSS layout:

```typescript
function caretVisualEdge(ta: HTMLTextAreaElement): { first: boolean; last: boolean } {
    const pos = ta.selectionStart;
    const val = ta.value;
    const needsFirst = !val.slice(0, pos).includes("\n");
    const needsLast  = !val.slice(pos).includes("\n");
    // Fast path: physical newlines settle it without DOM measurement.
    if (!needsFirst && !needsLast) return { first: false, last: false };

    // Mirror div replicates textarea layout so the browser word-wraps
    // text identically. The zero-width-space span marks the caret position;
    // its offsetTop is the visual line's Y offset within the content area.
    const cs = window.getComputedStyle(ta);
    const baseCss = /* ... copy font, padding, width, white-space:pre-wrap ... */;
    const measureY = (text: string): number => { /* ... */ };

    const caretY = measureY(val.slice(0, pos));
    return {
        first: needsFirst && caretY <= measureY(""),      // Y matches line 0
        last:  needsLast  && caretY >= measureY(val),     // Y matches last line
    };
}
```

**Key design decisions:**
- `\n` fast-path: avoids any DOM mutation when the answer is already known from
  character data (physical newlines). The mirror-div path only runs for single-
  paragraph (no-`\n`) text that might soft-wrap.
- Compare caretY to `measureY("")` (the baseline Y at the start of content —
  equivalent to `paddingTop`) rather than `=== 0`, so the check is
  padding-agnostic.
- Compare caretY to `measureY(val)` (Y at end of text) for last-line detection,
  using `>=` for floating-point safety.
- At most 3 DOM elements are created and immediately removed per ArrowUp/ArrowDown
  event. Cost is negligible at human typing rates.

---

## Best Practices

### For textarea keyboard navigation with history

1. **Never use character-counting alone to infer visual line position.** A
   `<textarea>` breaks lines both at `\n` characters AND at word-wrap boundaries
   (CSS `white-space: pre-wrap; word-wrap: break-word`). A string with no `\n`
   can still span 10 visual rows if it is long enough.

2. **Use the mirror-div technique for visual caret position in textareas.**  
   `window.getSelection()` and `Range.getClientRects()` do not work on `<textarea>`
   elements — they only work on `contenteditable`. The mirror-div approach is the
   industry-standard alternative: clone the textarea's CSS into a `position:absolute`
   `div`, insert the text up to the caret position as `textContent`, append a
   zero-width-space `<span>` as the caret marker, and read `span.offsetTop`.

3. **Prefer `contenteditable` for rich chat inputs if multi-line editing matters.**
   `<textarea>` was designed for plain-text forms. Rich editors (Slack, Discord,
   Linear) use `contenteditable` or a headless ProseMirror/Lexical instance
   because `window.getSelection()` gives pixel-accurate caret coordinates for
   free. If the composer ever needs syntax highlighting, mention-autocomplete, or
   other inline decorations, migrating to `contenteditable` removes the need for
   the mirror-div workaround entirely.

4. **Guard physical-newline fast paths before expensive DOM measurements.**  
   When the user HAS typed `\n`, the simple `includes("\n")` check is both correct
   and zero-cost. Reserve the DOM measurement for the ambiguous soft-wrap case.

5. **Write unit tests for the edge cases** (`empty`, `single-line`, `multi-physical-line`,
   `single-long-paragraph-that-soft-wraps`) at the time of initial implementation,
   not after a bug is reported. The `useHistoryPagination.test.ts` file in this
   repo shows the pattern: `createSignal` stubs, jsdom fixtures, and synchronous
   event dispatch.

---

## Timeline

| Time | Event |
|------|-------|
| Sometime before 2026-06-23 | `caretOnFirstLine` implemented with `includes("\n")` shortcut |
| 2026-06-23 | User reports ArrowUp jumps to previous message while cursor is on line 4 of a soft-wrapped composer |
| 2026-06-23 | Root cause identified: `\n` scan cannot detect visual line position |
| 2026-06-23 | Fix implemented: mirror-div `caretVisualEdge()` helper replaces the `\n` scan |

---

## What Went Well

- The existing guard structure (`caretOnFirstLine` / `caretOnLastLine`) was
  correct in intent — the bug was only in the implementation of those booleans.
- Fix is surgical: one helper function, two one-line replacements. No change to
  the history state machine or the surrounding keyboard handler.
