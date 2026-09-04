# Report: hover-to-peek time/token panel is missing on expanded tool calls

**Status:** implemented — fix + updated tests in PR #2972.
**Date:** 2026-09-04
**Author:** agent3
**Repo state:** `agentmuxai/agentmux` main @ `b2fbf8bd1`
**Trigger:** User report — the hover-to-peek panel (pinned to the right,
tracks the mouse vertically, shows exact time + token estimate) works on
agent thinking text, but not on the body/preview of tool calls.

---

## 1. Confirmed: real, and scoped to exactly one condition

All three document-node kinds that use the shared `PeekOverlay` +
`useNodePeek()` infrastructure were compared directly:

| Component | `show=` condition |
|---|---|
| `MarkdownBlock.tsx:146` (thinking/assistant text) | `isPeeking() && (peekTimeText() != null \|\| peekEstimateText() != null)` |
| `UserMessageBlock.tsx:219` (regular user input) | `isPeeking() && (peekTimeText() != null \|\| peekEstimateText() != null)` |
| `ToolBlock.tsx:493` (tool calls) | `isPeeking() && hasAnyPeekContent() && !expanded()` |

**Only `ToolBlock.tsx` adds `&& !expanded()`.** Neither `MarkdownBlock`
nor `UserMessageBlock` has any equivalent "already showing its body, so
suppress the hover panel" condition — they show the time/token peek on
hover unconditionally, regardless of any expand/pin state.

`expanded()` is true whenever a tool call's panel is showing its body —
pinned open by the user, auto-expanded (running / pending-approval /
just-completed hold-open), or manually toggled open. In every one of
those states, hovering the tool call currently shows **no** peek panel at
all — confirmed by reading `ToolBlock.tsx:314` (`panelMode()`) and the
panel's own render at `:517-535`, which renders only `<ToolBlockOverlay>`
(the command/output/diff body) with no time or token display anywhere in
it.

## 2. Why this happened (not a regression — a deliberate, now-stale call)

The suppression is original design intent, per the comment at
`ToolBlock.tsx:487-492`:

> "Suppressed once the panel is already expanded, since the command/time
> are visible in context there — same condition the old Tooltip-based
> version used."

This is **half right, half wrong**, and the wrong half is what the user
is hitting:

- `cmdText()` (the bare command) genuinely *is* redundant once
  expanded — the same command is visible in the panel body
  (`ToolBlockOverlay`).
- `peekTimeText()` (exact timestamp + "time ago") and `peekEstimateText()`
  (token estimate) are **not** shown anywhere in the expanded panel body.
  The comment's claim that "time... are visible in context there" does
  not hold — `ToolBlockOverlay` renders tool output, not a timestamp or
  token count. This was true at the time of the original (pre-Portal)
  Tooltip implementation this comment cites, or was simply wrong even
  then; either way, on current `main` it is not accurate.

Net effect: the moment a tool call's panel opens for any reason (running,
pending approval, pinned, or just finished and held open), the ONLY
surface that ever shows that node's time/token info becomes permanently
unavailable, with no alternate way to see it, for exactly as long as the
panel stays open — while `MarkdownBlock`/`UserMessageBlock` never lose
that surface regardless of their own expand states.

## 3. Fix

Keep suppressing `cmdText` when expanded (that part of the original
rationale is correct), but stop suppressing the whole overlay — let
`peekTimeText`/`peekEstimateText` show regardless of `expanded()`:

```tsx
<PeekOverlay show={isPeeking() && hasAnyPeekContent()} rowEl={peekRowEl}>
    <Show when={peekTimeText()}>
        <div class="agent-node-peek-tooltip-meta">{peekTimeText()}</div>
    </Show>
    <Show when={peekEstimateText()}>
        <div class="agent-node-peek-tooltip-meta">{peekEstimateText()}</div>
    </Show>
    <Show when={cmdText() && !expanded()}>
        <div class="agent-node-peek-tooltip-body">{cmdText()}</div>
    </Show>
</PeekOverlay>
```

- `show=` drops `&& !expanded()` entirely — the peek now appears on hover
  regardless of panel state, matching `MarkdownBlock`/`UserMessageBlock`.
- `hasAnyPeekContent()` (`:360-362`) already checks `cmdText() !== "" ||
  props.node.timestamp != null || peekEstimateText() != null` — unchanged,
  since a node with only a command and no timestamp/estimate should still
  peek-content-gate correctly (rare, but the existing logic already
  handles it).
- The `cmdText` `<Show>` inside the overlay body gets `&& !expanded()`
  moved onto it specifically, so the one genuinely-redundant line (the
  bare command, already visible in the open panel) stays hidden while
  expanded, but time/estimate now always show.

This is a minimal, targeted change — one condition moved from the
overlay's `show` gate to the one child that was actually redundant. No
change to `MarkdownBlock.tsx`, `UserMessageBlock.tsx`, `PeekOverlay.tsx`,
or `useNodePeek.ts` — the shared infrastructure (including the mouse-Y
tracking behavior from `SPEC_PEEK_OVERLAY_MOUSE_Y_TRACKING_2026_09_03.md`)
already works correctly for every node kind; `ToolBlock.tsx` alone had
its own extra gate.

## 4. What this does not change

- Collapsed tool calls: already worked, unaffected.
- Thinking/message blocks, user messages: already worked, unaffected.
- The peek's positioning/mouse-tracking/pointer-gap behavior
  (`PeekOverlay.tsx`): unchanged, this report is purely about the `show`
  condition and one child's visibility in `ToolBlock.tsx`.

## 5. Testing plan

- Existing test `ToolBlock.test.tsx` — "command tooltip > expanded
  (pinned): hovering the name shows no overlay — command is visible in
  the panel already" currently asserts the OLD (buggy) behavior and will
  need updating to assert the overlay DOES show, but WITHOUT the command
  body line, once pinned/expanded.
- New test: hovering an expanded/pinned tool call shows
  `peekTimeText`/`peekEstimateText` but not `cmdText`.
- Manual: pin a tool call open, hover it, confirm the time/token panel
  now appears (tracking the mouse per the existing mouse-Y behavior) and
  does not duplicate the command text already shown in the body.
