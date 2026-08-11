# SPEC: Copy Button False-Positive Checkmark Fix

**Date:** 2026-08-10
**Status:** Implemented
**Area:** Frontend — shared `CopyButton` element (code blocks, connection-status error copy)
**Severity:** P2 — misleading UI, masks a real clipboard failure

---

## Problem

Clicking any copy button in the app (code block copy, connection-status error
copy) always shows a green checkmark, even when the underlying clipboard write
fails. A user testing clipboard integration reported: clicking copy on a code
block shows the checkmark, but pasting into Notepad produces nothing — the
button gave no indication anything was wrong.

## Root Cause

**`frontend/app/element/copybutton.tsx`** (`CopyButton`) is a shared element
used by:
- `frontend/app/element/markdown-codeblock.tsx` — code block copy button
  (`handleCopy`, `async`, calls `clipboardWriteText`)
- `frontend/app/block/blockframe.tsx` — connection-status error copy
  (`handleCopy`, also `async`)

Both callers pass an **async** `onClick` handler that performs the actual
clipboard write via `frontend/util/clipboard.ts` → CEF IPC `write_clipboard` →
`agentmux-cef/src/commands/clipboard.rs` (OS clipboard).

The button's own click handler, however, was fire-and-forget with respect to
that async work:

```tsx
// BEFORE
const handleOnClick = (e: MouseEvent) => {
    if (isCopied()) return;
    setIsCopied(true);           // shown immediately, unconditionally
    // ...timeout bookkeeping...
    if (onClick) {
        onClick(e);               // async result never awaited or checked
    }
};
```

`setIsCopied(true)` (which drives the checkmark icon) ran synchronously,
*before* `onClick(e)` was even invoked, let alone before its promise
resolved. Whether the underlying `writeText()` call succeeded, rejected, or
was never reached (e.g. an IPC error, a stale host binary missing the
command, an OS-level clipboard failure) had **zero effect on the button's
displayed state**. The checkmark was not a signal of success — it fired
unconditionally on click.

This is orthogonal to whether the clipboard write itself works in any given
environment; the bug is that the UI can't tell the user either way.

## Fix

**File:** `frontend/app/element/copybutton.tsx`

Make the click handler `async`, `await` the caller's `onClick`, and only
show the checkmark if it resolves without throwing. On rejection, show a
distinct error state (red triangle-exclamation icon, title changed to "Copy
failed — see console") and log the error via `console.error` so the actual
underlying error (e.g. the rejected `invokeCommand` error) is visible for
diagnosis.

```tsx
// AFTER
const handleOnClick = async (e: MouseEvent) => {
    if (isCopied()) return;
    // ...clear any pending reset timeout...
    try {
        await onClick?.(e);
        setIsError(false);
        setIsCopied(true);
    } catch (err) {
        console.error("copy failed:", err);
        setIsCopied(false);
        setIsError(true);
    }
    // ...schedule state reset after 2s...
};
```

`CopyButtonProps.onClick` type widened from `(e: MouseEvent) => void` to
`(e: MouseEvent) => void | Promise<void>` to reflect that both current
callers pass async functions.

**File:** `frontend/app/element/copybutton.scss`

Added an `.error` variant (reuses the existing `--error-color` theme token,
same pattern as the existing `.copied` variant using `--success-color`).

## Non-Goals / Follow-up

- **Diagnosing why a given clipboard write fails in a specific environment**
  is out of scope here — this fix only makes failures observable. The
  underlying `write_clipboard` IPC path
  (`frontend/util/clipboard.ts` → `agentmux-cef/src/ipc.rs:400` →
  `agentmux-cef/src/commands/clipboard.rs`) was independently reviewed and
  looks structurally correct (standard Win32 `GlobalAlloc`/`SetClipboardData`
  idiom, present on `main` since April 2026, not a recent regression). If a
  failure persists after this fix, the console error message it now surfaces
  is the next debugging input.
- **Tool-call preview panels have no copy button at all** (`ToolBlockOverlay.tsx`
  / `ToolOverlayLog.tsx` / `ToolBlock.tsx`) — this was never built (the prior
  action bar there, `ToolOverlayActions.tsx`, only had pane/window actions and
  was removed as dead code in #1991). That's new-feature work, not covered by
  this fix, and is tracked separately.

## Files Changed

```
frontend/app/element/copybutton.tsx    (async handler, error state)
frontend/app/element/copybutton.scss   (.error style variant)
```

## Testing

- `npx tsc --noEmit` — clean, no type errors introduced.
- Manual: hover a chat code block, click copy, verify checkmark only appears
  after a successful write; verify pasted content matches; verify a forced
  failure (e.g. temporarily throwing in the write path) shows the red error
  icon and logs to console instead of a false checkmark.
