# Composer: Shift+ArrowUp triggers history recall before the top line is fully selected

**Date:** 2026-08-11
**Status:** Implemented — `AgentFooter.tsx`. The implementation simplified
further than §4 proposed: once the requirement is "true absolute position,"
the mirror-div visual-row measurement becomes unnecessary entirely (position
0 is always visual row 0 regardless of wrapping) — the whole `caretVisualEdge`
function was replaced with a two-line pure position check,
`caretAtSelectionEdge`, rather than layering a position check on top of the
existing measurement. See the function's own doc comment for the reasoning.
**Owner:** Agent3
**Area:** Agent pane composer (`AgentFooter.tsx`) — sent-message history
recall vs. text selection

---

## 1. Problem

In the Agent pane composer, selecting multi-line text and repeatedly pressing
Shift+ArrowUp to extend the selection upward behaves correctly line-by-line
until it reaches the topmost line, where only part of that line ends up
selected (expected — the browser lands the selection focus at whatever
column the shift+up sequence started from, which may fall mid-way through a
shorter top line). Pressing Shift+ArrowUp **again** at that point should
extend the selection to cover the rest of the top line (snap to column 0),
matching standard textarea behavior when there's no line above to move to.
Instead, it immediately triggers **sent-message history recall** — the
composer's content is replaced with the previously sent message, destroying
the in-progress selection.

**Desired behavior:** history recall should only trigger once the selection
already covers the true start of the content (column 0 of line 0) — one
keystroke later than it currently does.

## 2. Current behavior (root cause)

### 2.1 The handler

`AgentFooter.tsx:860-895` — the combined ArrowUp/ArrowDown handler for
queued-message un-queue + sent-message history recall:

```js
860  if (textareaRef && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
861      const empty = textareaRef.value.length === 0;
862      const navigating = histPos < sentHistory.length;
863      const { first: caretOnFirstLine, last: caretOnLastLine } = caretVisualEdge(textareaRef);
...
879      if (e.key === "ArrowUp" && histPos > 0 && caretOnFirstLine) {
880          if (!navigating) histDraft = textareaRef.value;
881          histPos--;
882          e.preventDefault();
883          setComposerValue(sentHistory[histPos]);
884          return;
885      }
```

`caretOnFirstLine` is the sole gate deciding "the user can't go up any
further in the textarea, so treat this ArrowUp as history navigation
instead." No `e.shiftKey` branch exists anywhere in this block (confirmed —
`grep shiftKey AgentFooter.tsx` only matches Enter and Ctrl/Cmd+Z elsewhere
in the file) — plain ArrowUp and Shift+ArrowUp share the exact same gate.

### 2.2 `caretOnFirstLine` checks the wrong thing: line, not column

`caretVisualEdge()` (`AgentFooter.tsx:376-409`):

```js
376  function caretVisualEdge(ta: HTMLTextAreaElement): { first: boolean; last: boolean } {
377      const pos = ta.selectionStart;
378      const val = ta.value;
379      const needsFirst = !val.slice(0, pos).includes("\n");
...
404      const caretY = measureY(val.slice(0, pos));
405      return {
406          first: needsFirst && caretY <= measureY(""),   // matches baseline (first visual row)
```

This answers "is the active position on visual row 0" — it does **not**
check "is the active position at absolute offset 0 (the true start of
content)." The moment the selection's moving end lands *anywhere* on the top
visual line — including mid-line, which per §1 is the normal, expected
result of the shift+up sequence's own line-by-line extension — `first`
already evaluates `true`. The very next ArrowUp then satisfies line 879's
`caretOnFirstLine` and fires history recall a keystroke early, before the
browser ever gets to extend the selection the rest of the way to column 0.

This is a distinct bug from the one the 2026-06-23 retro
(`docs/retro/RETRO_COMPOSER_ARROWKEY_HISTORY_SOFT_WRAP_2026_06_23.md`) fixed
in this exact function: that retro corrected line-vs-line detection (soft-wrapped
visual rows vs. physical `\n`), not line-vs-column. Its own "What Went Well"
section notes "the existing guard structure... was correct in intent — the
bug was only in the implementation of those booleans," which is why this
deeper column-level gap was never surfaced at the time — that statement was
true for the soft-wrap bug, but the boolean itself (`caretOnFirstLine`
meaning "on line 0") is the wrong question for the selection-extension case;
it needs to mean "at column 0 of line 0."

### 2.3 A second, independent bug in the same line: `selectionDirection` is never checked

`caretVisualEdge` always reads `ta.selectionStart` (line 377), regardless of
`ta.selectionDirection`. `grep -r selectionDirection frontend/` returns no
matches anywhere in the frontend — it's never referenced.

For a selection where the **anchor is before the focus** (`selectionDirection
=== "forward"` — e.g. the user shift-clicked or drag-selected downward from
an earlier point, then switched to Shift+ArrowUp to extend further), the end
that Shift+ArrowUp actually moves is `selectionEnd`, not `selectionStart`.
`caretVisualEdge` would measure the **fixed anchor** (`selectionStart`)
instead of the caret that's actually moving, making `caretOnFirstLine`
answer a question about the wrong end of the selection entirely.

The common repro path in §1 (extending a collapsed cursor upward via
Shift+ArrowUp from the start) happens to work correctly *by accident*: a
collapsed cursor has `selectionStart === selectionEnd`, so there's no
anchor/focus ambiguity until the first Shift+ArrowUp actually creates a
selection — and because that first extension is upward, the resulting
selection's moving end (backward direction) does correspond to
`selectionStart`. A selection that starts in the forward direction hits the
second bug independently of the first.

### 2.4 Non-shift ArrowUp shares the same over-eager gate

Since there's no `e.shiftKey` branch, a plain (non-shift) ArrowUp landing
mid-line on the top visual line *also* triggers history recall one keystroke
early, before the browser's native "ArrowUp on the top line moves the caret
to absolute position 0" behavior gets to run. This case doesn't have the
selection-extension nuance (a collapsed cursor's `selectionStart ===
selectionEnd` always), so the correct condition here is simpler: gate on
`textareaRef.selectionStart === 0` directly, no visual-row measurement
needed at all for this specific case.

### 2.5 User-visible impact of firing early

`setComposerValue()` (`AgentFooter.tsx:562-575`) fully replaces the textarea
content and collapses the selection to the end
(`textareaRef.setSelectionRange(text.length, text.length)`, line 571). So
when history recall fires prematurely, the user doesn't just fail to extend
their selection — their in-progress selection is destroyed outright and
replaced with a different message's text, cursor at the end of it. This is
what makes the bug disruptive rather than merely inconvenient.

## 3. Goal & requirements

1. **Shift+ArrowUp must fully select the top line before history recall can
   trigger.** History recall should only fire on the ArrowUp press *after*
   the selection's moving end is already at absolute offset 0.
2. **Plain ArrowUp must reach true column 0 before history recall can
   trigger**, not just "visual row 0" — same underlying fix, simpler
   condition (no selection direction to resolve, per §2.4).
3. **Correct for both selection directions.** A forward-direction selection
   (anchor before focus) extended upward via Shift+ArrowUp must gate on
   `selectionEnd`, not `selectionStart` — see §2.3.
4. **No regression to the existing soft-wrap fix** (2026-06-23 retro) — the
   mirror-div visual-row measurement for genuinely ambiguous soft-wrapped
   text must still work; this fix adds a column check on top of it, not a
   replacement for the visual-row logic itself where it's still needed.
5. **ArrowDown/last-line symmetry**: `caretOnLastLine` (`AgentFooter.tsx:889`)
   has the mirror-image version of this bug for Shift+ArrowDown extending a
   selection toward the end of a multi-line composer, gated on `navigating`
   rather than being able to spontaneously enter history mode — lower
   severity (only reachable while already navigating history) but should be
   fixed the same way for consistency, in the same change.

## 4. Proposed fix

### 4.1 Resolve the "moving end" once, correctly, for both keys

Add a small helper (or inline at the call site) that resolves which offset
is the one Shift+ArrowUp/Down actually moves, given `selectionDirection`:

```js
function activeSelectionEdge(ta: HTMLTextAreaElement): number {
    // "backward": anchor is selectionEnd, focus (the moving end) is
    // selectionStart. "forward" or "none" (collapsed, or same-direction
    // ambiguous): focus is selectionEnd. A collapsed cursor has
    // selectionStart === selectionEnd, so this is a no-op for the
    // non-shift case either way.
    return ta.selectionDirection === "backward" ? ta.selectionStart : ta.selectionEnd;
}
```

### 4.2 Require true column 0, not just visual row 0

`caretVisualEdge` (or its call site) needs the resolved edge from §4.1
instead of the raw `ta.selectionStart` at line 377, AND needs an additional
`=== 0` check layered on top of the existing visual-row check — the
visual-row check alone (needed for soft-wrap correctness per the June retro)
answers "is this row 0," not "is this absolute position 0":

```js
const edge = activeSelectionEdge(ta);
const onFirstVisualRow = /* existing caretVisualEdge "first" logic, using `edge` instead of ta.selectionStart */;
const first = onFirstVisualRow && edge === 0;
```

Symmetric change for `last`, using the requirement "at the true end of
content" (`edge === val.length`) instead of just "last visual row," per
requirement 5.

### 4.3 Keep the non-shift case simple

Per §2.4, non-shift ArrowUp doesn't need the visual-row measurement path at
all for the *history-trigger decision* — `selectionStart === 0` (equivalently
`activeSelectionEdge(ta) === 0` for a collapsed cursor, since anchor==focus)
is sufficient and cheaper. Whether it's worth special-casing this to skip
the mirror-div measurement, or just always going through the same
`first`/`last` computation from §4.2 uniformly for both shift and non-shift
(simpler code, negligible extra cost per the June retro's own note that the
mirror-div path is "negligible... at human typing rates"), is an
implementation choice — recommend the latter for less branching, unless
profiling shows otherwise.

## 5. Non-goals / out of scope

- The queued-message un-queue gesture (`AgentFooter.tsx:866-873`, only
  reachable on an empty composer) — unaffected, `empty` already implies
  `selectionStart === selectionEnd === 0`.
- Any change to the history state machine itself (`histPos`, `sentHistory`,
  `histDraft`) — this is purely about when the existing transitions are
  allowed to fire.
- Migrating the composer off `<textarea>` to `contenteditable` — floated as
  a "prefer contenteditable" best practice in the June retro for unrelated
  reasons (rich editing features), not needed to fix this.

## 6. Testing gap

No existing test covers Shift+ArrowUp / selection-extension behavior —
`AgentFooter.test.tsx` has no `shiftKey`-related ArrowUp/ArrowDown cases
(confirmed via grep). The June 2026 retro's own best-practices list
explicitly calls for testing `empty`, `single-line`, `multi-physical-line`,
and `single-long-paragraph-that-soft-wraps` cases "at the time of initial
implementation, not after a bug is reported" — this bug is a direct instance
of that gap recurring for a case (selection extension) the original tests
never covered either. The fix should add coverage for: collapsed-cursor
Shift+ArrowUp reaching column 0 before recall fires; a forward-direction
selection (§2.3) extended upward; plain ArrowUp on a mid-line top-row
position; and the symmetric ArrowDown/last-line cases.
