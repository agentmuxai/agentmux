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

## 2. Fix: let title claim natural width first, tail take the leftover, title shrink only as a last resort

```scss
.agent-activity-title {
    flex: 0 1 auto;
    min-width: 0;
    /* max-width: 40% removed */
    ...
}
```

- `flex-grow: 0` — text doesn't need to stretch to fill space; sizing to
  content is correct once there's no artificial cap.
- `flex-shrink: 1` (was `0`) + `min-width: 0` (was unset, which defaults
  to `auto` — a flex item's own `min-width: auto` floors it at its
  content's min-content width, which for a `white-space: nowrap` span is
  its FULL text width, so shrink would never actually engage without
  this) — together these let the title shrink, and its ellipsis fire,
  but only when the row is genuinely out of room.

Why this doesn't just make the title always show in full and break the
ellipsis entirely: CSS flexbox distributes negative free space (when
content overflows) proportionally to each item's `flex-shrink × flex-
basis`. `.agent-activity-tail` already has `flex-basis: 0` — its
contribution to that weighted distribution is `1 × 0 = 0`, so in a
genuine overflow it absorbs none of the forced shrinkage (it's already
sized from zero, growing only into space nothing else claims). The
title's `flex-basis` is its actual content width, so it's the only item
with a nonzero weight — meaning **when space is tight, 100% of the
required shrink lands on the title**, exactly the desired "shrink only
when there's truly no space left" behavior, and when space is NOT tight,
title simply renders at its natural content width with no cap.

Sigil, elapsed, remaining, and the stop/dismiss buttons stay `flex: 0 0
auto` — fixed-content chrome that was never the problem.

### 2.1 Same bug, same fix, in the in-transcript persistent shell block

`.agent-shell-title` (`_shell-node.scss:92-100`) is byte-for-byte the
same `flex: 0 0 auto; max-width: 40%;` pattern, in
`PersistentShellBlock.tsx` — the in-transcript rendering of a running
shell (distinct from the dock's summary row, but intentionally styled as
its structural twin: same sigil/title/elapsed/tail/stop layout, per the
`_shell-node.scss` section grouping and `ActivityRow.tsx`'s own comment
that it shares "the same cap + renderer as PersistentShellBlock"). Fixing
only the dock's copy would newly make the two visibly inconsistent
(dock shows a title in full, the in-transcript block for the exact same
process still clips at 40%). Both get the identical fix:
`.agent-shell-title` → `flex: 0 1 auto; min-width: 0;`, no `max-width`.
`.agent-shell-live-tail` (`:109-118`) already has `flex: 1 1 0; min-width:
0` — unchanged, same reasoning as `.agent-activity-tail`.

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
