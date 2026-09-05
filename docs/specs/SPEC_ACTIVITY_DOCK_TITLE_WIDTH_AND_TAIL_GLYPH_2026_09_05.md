# SPEC — Activity dock: title over-truncates; tail glyph renders wrong near the time

**Status:** proposed → implementing
**Date:** 2026-09-05
**Author:** agent3
**Trigger:** User report — the pinned activity dock's docked items for
long-running processes truncate their title too aggressively (leaving
visible free space in the row), and a symbol renders as an invalid
character near the elapsed time.
**Related:**
- `frontend/app/view/agent/components/ActivityRow.tsx` (the dock row)
- `frontend/app/view/agent/components/PersistentShellBlock.tsx` (the
  in-transcript persistent shell block — structurally identical chrome,
  see §3)
- `frontend/app/view/agent/styles/_shell-node.scss` (both components'
  shared CSS section)
- `docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md` (original
  dock spec, §4 chrome)

---

## 1. Truncation: `max-width: 40%` is a fixed ceiling, not a real constraint

`.agent-activity-title` (`_shell-node.scss:252-261`):

```scss
.agent-activity-title {
    flex: 0 0 auto;
    font-size: 12px;
    font-weight: 600;
    color: var(--main-text-color);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 40%;
}
```

`flex: 0 0 auto` (no grow, no shrink, basis = content) plus a hardcoded
`max-width: 40%` means the title box is **always** capped at 40% of the
row's width, regardless of what the row's other children actually need.
The one flexible sibling, `.agent-activity-tail` (`:282-292`,
`flex: 1 1 0; min-width: 0;`), absorbs whatever's left over — but `tail()`
(`ActivityRow.tsx:124-160`) is frequently empty (e.g. a subagent with
`event_count === 0`, or a shell/tool with no stdout/stderr yet), in which
case its `<Show>` never mounts and that space is simply never
reclaimed — not by the title, not by anything. Result: a long title
ellipsizes at 40% even when the sigil, elapsed clock, and stop/dismiss
button leave most of the row visibly empty. This matches the report
exactly ("truncation is too much, leaving plenty of space free").

## 2. Fix: a proportional flex-basis, not an absolute cap

**Revised once during review (Codex P2, correct — see §2.2).** Final
version:

```scss
.agent-activity-title {
    flex: 1 1 40%;
    min-width: 0;
    /* max-width: 40% removed */
    ...
}
.agent-activity-tail {
    flex: 1 1 60%;
    min-width: 0;
    ...
}
```

Both items keep the same 40/60 split the old `max-width: 40%` implied,
but as a `flex-basis` rather than a hard ceiling — the difference matters
in exactly the case the report is about:

- **Tail absent or short:** title is the only item actually competing for
  space (or the least-hungry one), so `flex-grow: 1` lets it claim the
  freed space and render in full — fixing the reported dead-space bug.
  Growing a text span's box beyond its own content is invisible (no
  border/background, left-aligned text) whenever the content already
  fits, so this doesn't distort short titles either.
- **Both genuinely long:** CSS flexbox distributes negative free space
  proportionally to each item's `flex-shrink × flex-basis`. With title at
  `40%` and tail at `60%`, both have a **nonzero** weight, so an overflow
  shrinks both roughly in their 40/60 share — title's ellipsis engages,
  but the tail keeps some visible width too, instead of one side taking
  100% of the squeeze.

`min-width: 0` on both (title's is new) overrides the flex-item default
of `min-width: auto`, which would otherwise floor a `white-space: nowrap`
span at its full text width and prevent shrinking — and hence the
ellipsis — from ever engaging at all.

Sigil, elapsed, remaining, and the stop/dismiss buttons stay `flex: 0 0
auto` — fixed-content chrome that was never the problem.

### 2.1 Same bug, same fix, in the in-transcript persistent shell block

`.agent-shell-title`/`.agent-shell-live-tail` (`_shell-node.scss:92-100`,
`117-133`) are the byte-for-byte same `flex: 0 0 auto; max-width: 40%` /
`flex: 1 1 0` pattern, in `PersistentShellBlock.tsx` — the in-transcript
rendering of a running shell (distinct from the dock's summary row, but
intentionally styled as its structural twin: same sigil/title/elapsed/
tail/stop layout, per the `_shell-node.scss` section grouping and
`ActivityRow.tsx`'s own comment that it shares "the same cap + renderer
as PersistentShellBlock"). Fixing only the dock's copy would newly make
the two visibly inconsistent. Both get the identical `flex: 1 1 40%` /
`flex: 1 1 60%` fix.

### 2.2 What was tried first, and why it changed

**Attempt 1:** `.agent-activity-title { flex: 0 1 auto; min-width: 0; }`
(flex-basis = content width) paired with the pre-existing
`.agent-activity-tail { flex: 1 1 0; }` (flex-basis = 0). This fixes the
reported "wasted space when tail is short/absent" case, but:

- **Codex P2 (correct):** when title's own content is long enough to
  overflow the row on its own — regardless of the tail — the shrink-
  distribution weight is `flex-shrink × flex-basis`: title's is
  `1 × (its full content width)`, tail's is `1 × 0 = 0`. Title absorbs
  **100%** of the required shrink; tail, having zero weight, gets none —
  it was already sized from zero and simply stays at 0. A genuinely long
  title therefore squeezed the tail's status/live-output text out
  entirely, even in a pane far wider than the 400px container-query
  breakpoint that intentionally hides the tail on narrow panes. The old
  `max-width: 40%` cap accidentally guaranteed the tail some space in
  exactly this case (title could never claim more than 40% regardless of
  its own content length) — attempt 1 traded that guarantee away.

**Final (this spec):** give both a nonzero proportional `flex-basis`
(§2) instead of one absolute (title) and one zero (tail) — this keeps
attempt 1's fix for the reported case (short/absent tail) while
restoring a guaranteed nonzero share for the tail when both are
genuinely competing for space.

## 3. The glyph: `↳` (U+21B3) has weak font coverage; `→` (U+2192) is already proven safe here

Both tail spans prefix their content with `↳ ` (DOWNWARDS ARROW WITH TIP
RIGHTWARDS, U+21B3):

```tsx
// ActivityRow.tsx:223
<span class="agent-activity-tail">↳ {tail()}</span>
// PersistentShellBlock.tsx:147
<span class="agent-shell-live-tail">↳ {lastLine()}</span>
```

Both spans render in the app's fixed/monospace font stack
(`font-family: var(--fixed-font, monospace)` → `"Hack", monospace`,
`_shell-node.scss:287`). U+21B3 sits in the general Arrows block but is a
much less commonly-included compound glyph (a combined "down then right"
arrow) than the basic directional arrows — Hack's own glyph set is
Latin/programming-symbol focused and does not reliably include it. When
the active font (and its short 2-entry fallback list, `"Hack",
monospace`) has no glyph for a character, the renderer falls through to
whatever font the OS substitutes for that one character — a different
typeface, weight, and baseline than the surrounding monospace text,
which reads as a wrong/garbled/"invalid" character sitting right next to
otherwise-normal text. This lines up with the report: it happens
specifically when a tail is showing (i.e. "sometimes" — whenever `tail()`
/ `lastLine()` is non-empty, not universally, since the `<Show>` gates on
that), and specifically "around the time" — the tail span sits
immediately after the elapsed/remaining spans in the row.

**Fix:** swap `↳` for `→` (RIGHTWARDS ARROW, U+2192) — a basic, near-
universally-supported arrow already used successfully elsewhere in this
exact file, in the exact same monospace context, with no reported
rendering issue: `subagentEventLine`'s `tool_use` case
(`ActivityRow.tsx:39`, `` `→ ${t.name}` ``), rendered inside
`.agent-tool-log-line` (also `var(--fixed-font)`). Reusing a codepoint
this codebase already relies on in the identical font stack is a safer
bet than guessing at a third glyph.

### 3.1 Same glyph exists elsewhere — out of scope here, flagged for a follow-up

`↳` also appears in `ToolBlock.tsx:474` (a tool call's own live-tail,
`.agent-tool-live-tail`, same `var(--fixed-font)` context) and
`PaneRow.tsx:97` (`.pane-row-tail`, `var(--font-mono)`). Same likely
risk, same fix would apply — not changed here since the report was
specifically about the dock/shell-block tail, and `PaneRow.tsx` has an
existing test (`PaneRow.test.tsx:26`) asserting the current glyph that
would need updating alongside it. Worth a follow-up pass for visual
consistency across the app, not required to resolve this report.

## 4. Non-goals

- No change to `.agent-activity-tail`/`.agent-shell-live-tail` themselves
  — already correctly configured (`flex: 1 1 0; min-width: 0`).
- No change to sigil/elapsed/remaining/stop/dismiss sizing.
- No change to `ToolBlock.tsx`/`PaneRow.tsx`'s own `↳` usage (§3.1).
- No change to the `@container agent-pane (max-width: 399px)` narrow-pane
  behavior that hides the tail entirely — unaffected by either fix.

## 5. Testing

- Manual: run a long-running shell/tool with a short tail (or none) and a
  long title — title should now render in full (or truncate only if the
  pane itself is narrow enough that title alone doesn't fit), not clip at
  a fixed 40% with visible dead space beside it.
- Manual: trigger a docked item with live tail output, confirm the arrow
  glyph renders as a normal monospace `→`, not a mismatched/garbled
  character.
- No existing test in `PersistentShellBlock.test.tsx` or
  `ActivityRow.countdown.test.tsx` asserts on the `max-width` CSS value
  or the `↳` character — confirmed via direct search, nothing to update.
