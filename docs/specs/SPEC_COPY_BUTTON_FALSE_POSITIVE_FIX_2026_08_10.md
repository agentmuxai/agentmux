# SPEC: Copy Button Silently Failing (Three Stacked Bugs)

**Date:** 2026-08-10
**Status:** Implemented, verified end-to-end (real OS clipboard content checked directly, not just UI state)
**Area:** Frontend `CopyButton` element, CEF host clipboard IPC (Windows), markdown code block text extraction
**Severity:** P1 — code block copy has been silently writing an empty string to the clipboard, likely since the feature was first wired up

---

## Problem

A user testing clipboard integration reported: clicking copy on a chat code
block shows the checkmark, but pasting into Notepad produces nothing.

This turned out to be **three independent bugs stacked on top of each
other**, each masking the next. Fixing only the first (the obvious one) was
not sufficient — the checkmark kept appearing and paste kept failing until
all three were found and fixed. Diagnosis for the second and third bugs used
direct OS-level verification (PowerShell `Get-Clipboard` /
`System.Windows.Forms.Clipboard`) rather than relying on the app's own UI
state, which is what caught them.

---

## Bug 1 — `CopyButton` showed the checkmark unconditionally

**File:** `frontend/app/element/copybutton.tsx`

The click handler set `isCopied(true)` (driving the checkmark) synchronously
on click, before the caller's **async** `onClick` (the actual clipboard
write) was even invoked, let alone before it resolved or rejected:

```tsx
// BEFORE
const handleOnClick = (e: MouseEvent) => {
    if (isCopied()) return;
    setIsCopied(true);           // shown immediately, unconditionally
    if (onClick) {
        onClick(e);               // async result never awaited or checked
    }
};
```

Both callers (`markdown-codeblock.tsx`'s code block copy, `blockframe.tsx`'s
connection-status error copy) pass async handlers. Whether the underlying
write succeeded, rejected, or errored had zero effect on the button's
displayed state — the checkmark was not a signal of anything.

**Fix:** made the handler `async`, `await` the caller's `onClick`, and only
show the checkmark if it resolves without throwing. On rejection, show a
distinct error state (red triangle-exclamation icon, title "Copy failed —
see console") and `console.error` the real error.

`CopyButtonProps.onClick` type widened from `(e: MouseEvent) => void` to
`(e: MouseEvent) => void | Promise<void>`. `copybutton.scss` gained an
`.error` variant (reuses `--error-color`, mirrors the existing `.copied`
pattern).

This fix was necessary to make failures *observable* at all, but on its own
did not fix the underlying paste failure — it only stopped hiding it.

---

## Bug 2 — Windows `SetClipboardData` return value was never checked

**File:** `agentmux-cef/src/commands/clipboard.rs`

```rust
// BEFORE
EmptyClipboard();
SetClipboardData(CF_UNICODETEXT as u32, hmem);   // return value discarded
CloseClipboard();
Ok(())                                            // always reports success
```

`SetClipboardData` returns `NULL` on failure. The Windows write path ignored
that and returned `Ok(())` unconditionally, so even a genuine OS-level
clipboard failure would round-trip back through IPC as success — the
frontend promise resolves, Bug 1's now-fixed button correctly shows a
checkmark, and still nothing useful reaches the clipboard.

**Fix:** check the return value; on `NULL`, capture
`std::io::Error::last_os_error()`, free `hmem` (ownership only transfers to
the system on success — the old code leaked `hmem` on this path too), and
return a real `Err` with the OS error message.

This fix turned out not to be the actual cause of the reported failure
(`SetClipboardData` was in fact succeeding), but it was a real, independent
correctness bug in its own right — silently discarding a checked Win32 API's
failure return is wrong regardless of whether it happened to bite here. It's
also what would have surfaced Bug 3's actual failure mode as a *visible*
error rather than a silent one, had Bug 3's failure mode been a rejected
write instead of a successful-but-empty one.

---

## Bug 3 (root cause) — code block text extraction always produced `""`

**File:** `frontend/app/element/markdown-codeblock.tsx`

After fixing Bugs 1 and 2, the checkmark still appeared and paste was still
empty. Direct inspection of the OS clipboard confirmed why:

```powershell
Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.Clipboard]::ContainsText()   # True
[System.Windows.Forms.Clipboard]::GetText().Length # 0
```

The `UnicodeText` clipboard format was present — the write path (Bugs 1 & 2)
was working correctly end-to-end — but the string being written was empty.

`CodeBlock`'s `getTextContent()` helper walked `children` assuming a
React-style tree (`typeof x === "string"`, `Array.isArray(x)`, or
`x.props.children`):

```tsx
// BEFORE
const getTextContent = (children: any): string => {
    if (typeof children === "string") return children;
    if (Array.isArray(children)) return children.map(getTextContent).join("");
    if (children && children.props && children.props.children) {
        return getTextContent(children.props.children);
    }
    return "";
};
```

But `frontend/app/element/markdown.tsx:17` builds this markdown renderer's
elements via `solid-js/h/jsx-runtime` (feeding `hast-util-to-jsx-runtime`).
Solid has no virtual DOM — this runtime creates **real DOM nodes** eagerly.
`children` passed into `CodeBlock` is an actual `<code>` `HTMLElement` (with
nested `<span>` highlight tokens from `rehypeHighlight`), not an object with
a `.props` property. None of `getTextContent`'s three branches ever matched
a real DOM node, so it always fell through to `return ""` — for every code
block, unconditionally. This is why the clipboard always ended up with valid
but empty text: the write path was never broken, the value being handed to
it always was.

**Fix:** stop trying to walk a tree shape that never existed at runtime.
Wrap the rendered `children` in a ref'd `<div class="codeblock-content">`
and read `.textContent` directly off the real DOM after render — robust
regardless of the JSX runtime's internal node representation:

```tsx
// AFTER
let contentRef: HTMLDivElement | undefined;
const getTextContent = (): string =>
    (contentRef?.textContent ?? "").replace(/\n$/, "");
// ...
<pre class="codeblock">
    <div class="codeblock-content" ref={contentRef}>{children}</div>
    <div class="codeblock-actions">...</div>
</pre>
```

The new wrapper `<div>` doesn't require any SCSS changes — `pre.codeblock
code { ... }` is a descendant selector (still matches at any depth), and
`.codeblock-actions` remains a direct sibling of the content div under the
same `position: relative` `<pre>`, so its `position: absolute` placement and
`:hover` reveal are unaffected. No other code depends on the internal DOM
structure of `pre.codeblock` (confirmed via repo-wide search).

### Edge case caught in review: mermaid blocks

The DOM-`.textContent` fix works for plain code blocks (highlight spans
don't change the underlying text), but `Code()` renders mermaid blocks as an
**SVG diagram**, not a `<code>` element — replacing the source text with
Mermaid's rendered output entirely. Reading `.textContent` off that DOM
picks up the diagram's own rendered `<text>` label nodes instead of the
mermaid chart source, so copy on a mermaid block would silently copy
garbled label text rather than the diagram source.

**Fix:** `Code()` now wraps the mermaid render in a `<div data-raw-code={text}>`
carrying the original source; `CodeBlock`'s `getTextContent()` checks for
that attribute first and only falls back to `.textContent` when it's absent:

```tsx
// Code(), mermaid branch
return (
    <div data-raw-code={text}>
        <ErrorBoundary fallback={<MermaidErrorFallback chart={text} />}>
            <Mermaid chart={text} />
        </ErrorBoundary>
    </div>
);

// CodeBlock's getTextContent()
const rawCode = contentRef?.querySelector<HTMLElement>("[data-raw-code]");
const text = rawCode ? (rawCode.dataset.rawCode ?? "") : (contentRef?.textContent ?? "");
```

---

## Why this shipped broken for so long

Bug 1 masked Bugs 2 and 3 completely — the checkmark always fired, so nobody
testing casually (checkmark-only, no actual paste-and-check) would have
caught that the clipboard was empty. `write_clipboard`/`read_clipboard` has
been on `main` since April 2026 (commit `b86c69429`) — this was not a recent
regression, just never actually exercised end-to-end.

---

## Files Changed

```
frontend/app/element/copybutton.tsx           (Bug 1: async handler, error state, pending-guard)
frontend/app/element/copybutton.scss          (Bug 1: .error style variant)
agentmux-cef/src/commands/clipboard.rs        (Bug 2: check SetClipboardData result)
frontend/app/element/markdown-codeblock.tsx   (Bug 3: DOM-ref-based text extraction, mermaid edge case)
.changesets/*.md                              (patch changeset for this PR)
```

## Non-Goals / Follow-up

- **Tool-call preview panels have no copy button at all** (`ToolBlockOverlay.tsx`
  / `ToolOverlayLog.tsx` / `ToolBlock.tsx`) — this was never built (the prior
  action bar there, `ToolOverlayActions.tsx`, only had pane/window actions and
  was removed as dead code in #1991). That's new-feature work, not covered by
  this fix, and is tracked separately.
- The macOS/Linux clipboard write paths (`pbcopy`/`wl-copy`/`xclip`/`xsel` in
  the same `clipboard.rs`) were not independently re-verified on those
  platforms — Bug 3 was a frontend bug affecting all platforms equally, and
  Bug 2's fix is Windows-specific code, but no macOS/Linux hardware was
  available to confirm end-to-end paste there.

## Testing

- `npx tsc --noEmit` — clean, no type errors introduced.
- Manual, verified end-to-end on Windows (`task dev`):
  1. Confirmed the failure directly against the OS clipboard (PowerShell
     `Get-Clipboard` / `System.Windows.Forms.Clipboard`) rather than trusting
     the app's own success UI — this is what caught Bugs 2 and 3 after Bug 1
     alone didn't fix the reported symptom.
  2. After all three fixes: clicked copy on a chat code block, checkmark
     appeared, pasted into Notepad — full code block content present and
     correct.
