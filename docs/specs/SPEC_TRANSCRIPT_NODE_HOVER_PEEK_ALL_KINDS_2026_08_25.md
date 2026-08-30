# Spec: hover-to-peek on EVERY transcript node kind, 50ms delay

**Status:** Implemented.

**Trigger (verbatim):** "they actually always need to fire. reduce delay for
50ms. Anytime you hover over the agent pane conversation history, there
should be hover text, because you are always over a node."

## 0. Context — the fourth pass at this feature

`SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md` (PR #2392) shipped a peek
tooltip (`PeekOverlay.tsx` — Portal-rendered, top-anchored, time + time-ago +
token estimate) on exactly three anchors: the tool-call name span, a
thinking-clump's header, and a regular user-input message row. That spec was
deliberately narrow — two prior specs (`node-timestamp-hover.md`,
`SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md`,
`SPEC_REMOVE_NODE_HOVER_STRIP_2026_06_15.md`) had each shipped and then
ripped out a generic per-row hover strip after user complaints about flicker
and "the row dances around my cursor" — so §2.5/§6 of the Aug-3 spec
explicitly scoped the feature down to three anchors and called "regular
assistant text, section headers... out of scope."

The user's follow-up request here reverses that scope decision: every node
in the transcript should show a peek on hover, not just three kinds. This is
safe to do now in a way the earlier, killed attempts weren't — `PeekOverlay`
is a Portal-rendered floating tooltip (`position: fixed`, sized off
`getBoundingClientRect()`) that never participates in document flow, so
widening its triggers doesn't reintroduce the earlier "row dances" bug (that
bug came from hover-driven *expand/collapse*, a layout-affecting state
change, not from a read-only floating overlay). No further scope walk-back
was needed here — the fix is additive coverage on the existing safe
mechanism, not a new mechanism.

## 1. What changed

### 1.1 Delay: 150ms → 50ms, centralized

The three existing implementations (`ToolBlock.tsx`, `MarkdownBlock.tsx`,
`UserMessageBlock.tsx`) each declared their own local `150` constant,
independently. Centralized to `PEEK_ENTER_DELAY_MS = 50` in
`components/hover-anchor.ts`, imported everywhere — one place to change,
not three to keep in sync.

### 1.2 Shared hook: `hooks/useNodePeek.ts`

Every implementation (old and new) hand-rolled the same
`isPeeking`/timer/`rowEl` boilerplate. Factored into `useNodePeek()`,
returning `{ isPeeking, rowEl, setRowEl, handlePeekEnter, handlePeekLeave }`.
`rowEl` is a signal (not a closure `let`), so `<PeekOverlay rowEl={rowEl}>`
needs no wrapper accessor at the call site.

### 1.3 Widened anchors — existing three node kinds

- **ToolBlock**: the peek anchor was a narrow `<span
  class="agent-tool-name-peek-anchor">` wrapping only the tool-name text.
  Moved the `handlePeekEnter`/`handlePeekLeave` calls onto the ROW's
  existing `onMouseEnter`/`onMouseLeave` (which already drove `userHolding`
  — "keep an already-expanded panel open while the mouse is still over
  it"), so hovering anywhere in the row triggers the peek, not just the name
  text. This couples the two behaviors: hovering a running tool now also
  engages `userHolding`, so if it completes while the cursor is stationary,
  the panel stays held open (correct — the user is visibly still reading
  it) and the peek stays correctly suppressed until a fresh hover, per the
  existing "peek is redundant once the panel is expanded" rule. See
  `ToolBlock.test.tsx`'s updated test for the exact before/after.
- **MarkdownBlock**: previously only thinking clumps got a peek (their
  anchor already wrapped the whole block). Regular (non-thinking) assistant
  text got NONE. Restructured so both branches share one anchor + overlay,
  varying only the `thinking-block` CSS class inside.
- **UserMessageBlock**: regular (non-startup) input already anchored on the
  full row — no widening needed, just the delay swap.

### 1.4 New peek coverage — nine node kinds that had none

| Node kind | Where | Content |
|---|---|---|
| `agent_message` | `AgentMessageBlock.tsx` | time + estimate. Never shows its own timestamp anywhere else, collapsed or expanded, so not gated on collapsed state. |
| `jekt_message` | `JektBubble.tsx` | time + estimate. Not gated on collapsed — the expanded view's timestamp line has no relative "ago", the peek adds that even when already expanded. |
| `shell` | `PersistentShellBlock.tsx` | time (from `spawnedAt`) + estimate + the raw `cmd` body. Suppressed while pinned open — command already visible in the panel header, same rule as ToolBlock. |
| `section` | inline in `DocumentRow.tsx` | time (if present — many historical nodes predate the field) + estimate(title). |
| `agent_error` | inline in `DocumentRow.tsx` | estimate only — `AgentErrorNode` carries no timestamp field at all. |
| `context_compacted` | inline in `DocumentRow.tsx` | time only — tokens/duration are already visible in the row; no free-text body to estimate. |
| `compaction_started` | inline in `DocumentRow.tsx` | time only, from `startedAt` (not `timestamp`). |
| `day_divider` | inline in `DocumentRow.tsx` | time only — the exact local-midnight instant; the visible label is already a human day name. |
| `session_outcome` | inline in `DocumentRow.tsx` | time + a body showing `attempted`/`actual` session ids — real debug info not surfaced anywhere else on this node. |

The six inline kinds share ONE `useNodePeek()` instance at the top of
`DocumentNodeBody` (the function that dispatches all of them) rather than
one each — safe because exactly one `<Show>` branch is ever mounted per
node, so only one of the six anchors actually exists in the DOM at a time.

### 1.5 The one deliberate exception: `history_link`

`HistoryLinkNode` is a render-time-only synthetic CTA row (`{ type:
"history_link", id: "history-link" }`) — no timestamp, no message, no
content field of any kind, and its full text ("Open Agent History →") is
already entirely visible without hovering. There is no data a peek could
add. Left with no anchor at all, and a comment in `DocumentRow.tsx`
documenting why — this is the one node kind that doesn't get the "always
fires" treatment, and it's a data-availability limit, not a scope
narrowing.

## 2. Files touched

```
New:
  frontend/app/view/agent/hooks/useNodePeek.ts
  frontend/app/view/agent/hooks/useNodePeek.test.ts
  frontend/app/view/agent/components/AgentMessageBlock.test.tsx
  frontend/app/view/agent/components/JektBubble.test.tsx
  frontend/app/view/agent/components/PersistentShellBlock.test.tsx

Modified:
  frontend/app/view/agent/components/hover-anchor.ts       (shared 50ms constant)
  frontend/app/view/agent/components/ToolBlock.tsx          (widened anchor, shared hook)
  frontend/app/view/agent/components/ToolBlock.test.tsx     (updated hover target + one behavior-change test)
  frontend/app/view/agent/components/MarkdownBlock.tsx      (regular text now peeks too)
  frontend/app/view/agent/components/MarkdownBlock.test.tsx
  frontend/app/view/agent/components/UserMessageBlock.tsx   (shared hook/delay)
  frontend/app/view/agent/components/UserMessageBlock.test.tsx
  frontend/app/view/agent/components/AgentMessageBlock.tsx  (new peek)
  frontend/app/view/agent/components/JektBubble.tsx         (new peek)
  frontend/app/view/agent/components/PersistentShellBlock.tsx (new peek)
  frontend/app/view/agent/virtualization/DocumentRow.tsx    (peek for 6 inline kinds + history_link exception comment)
  frontend/app/view/agent/virtualization/DocumentRow.test.tsx
```

## 3. Verification

- `tsc --noEmit`: clean.
- `vitest run frontend/app/view/agent`: 1426/1426 passing across 115 files
  (up from 1401 pre-change — 25 new tests: the hook + 3 new component test
  files + 7 new DocumentRow cases, plus edits to existing suites).
- `vitest run frontend/app/view/swarm`: 90/90 passing (no regression from
  reusing `AgentDispatch`-adjacent types).
- `task build:frontend`: production build succeeds (pre-existing chunk-size
  warnings only, unrelated).
- Not done: interactive/visual verification in a live running instance —
  same constraint as the Aug-3/Aug-19 specs (shared, already-live AgentMux
  instances on this machine; native Rust+CEF desktop app, not
  screenshot-able via browser tooling).
