# SPEC: Agent Pane Header — Name Precedence + Drop "continued" Chip

**Date:** 2026-06-29
**Status:** Draft
**Author:** AgentX
**Related:** `frontend/app/block/block.scss`, `frontend/app/block/blockframe.tsx`, `frontend/app/view/agent/agent-model.ts`, `frontend/app/view/agent/agent-view.tsx`, `frontend/app/view/agent/hooks/useHistoryPagination.ts`

---

## 1. Problems

Two issues in the agent pane's frame header:

1. **The activity summary cuts off the agent name.** When the header is narrow,
   the agent name truncates *before* the per-turn summary does — the opposite of
   what we want. The name is the primary identifier and should win the space
   contest.

2. **The "· continued 5h" chip has no useful meaning.** It reports how long ago
   the agent's session zone was last written before this pane mounted — an
   implementation detail that reads as noise to the user. Remove it.

## 2. Current behavior (why the name loses)

The header (`blockframe.tsx:457-509`) is a flex row with these relevant siblings:

| Region | Class | Contents | Flex |
|---|---|---|---|
| Name | `.block-frame-default-header-iconview` | icon + agent name (`viewName`) | `flex-shrink: 3` (`block.scss:96`) |
| Summary | `.block-frame-textelems-wrapper` | `viewText` elems: `term:activity` summary + continued chip | `flex: 1 2 auto` (`block.scss:222`) |

The name region shrinks at rate **3**, the summary region at rate **2**. Under
space pressure the name therefore collapses *faster* than the summary — the agent
name ellipsizes (down to `min-width: 17px`) while the summary keeps its width.
That is the reported bug.

`viewName` / `viewText` are produced by `AgentViewModel` (`agent-model.ts:129-165`):
`viewName` → `meta.agentName` (fallback "Agent"); `viewText` → `[ term:activity
summary, "· continued …" chip ]`.

The "continued" chip (`agent-model.ts:150-162`) renders when `continuedFromMsAtom`
is set and the gap ≥ 30 s. That atom is fed by `useHistoryPagination`'s
`onContinuationModts` callback (`agent-view.tsx:328`; `useHistoryPagination.ts:78,
321, 364`), formatted by `formatContinuationAgo` (`agent-model.ts:25`).

## 3. Goals

1. **Name precedence:** the agent name keeps its space; the activity summary is
   what shrinks/ellipsizes first. The name only truncates once the summary has
   collapsed to nothing.
2. **Remove** the "continued" chip and its now-dead plumbing.

## 4. Design

### 4.1 Name precedence (CSS only)

Invert the shrink priority between the two regions so the **summary yields first**:

- `.block-frame-default-header-iconview` — stop shrinking the name ahead of the
  summary. Set `flex-shrink: 0` (or a small value strictly **less** than the
  summary's), and keep a sensible `min-width: 0` + ellipsis so a pathologically
  long name still degrades gracefully rather than overflowing the row.
- `.block-frame-textelems-wrapper` — raise its `flex-shrink` (e.g. `flex: 1 100
  auto`) so it absorbs essentially all the squeeze. The inner `.term-activity`
  already has `overflow: hidden; text-overflow: ellipsis; white-space: nowrap`
  (`block.scss:166-171`), so it ellipsizes cleanly as it loses width.

**Trade-off (decided):** with the name at `flex-shrink: 0`, a very long agent name
can push the summary out entirely. That is the intended precedence — the name is
the identifier; the summary is ancillary. To avoid a name monopolizing the whole
header, optionally cap the name region with a `max-width` (e.g. `60%`) so the
summary always retains a sliver until the name itself needs to ellipsize. Pick
**one** of:

- **A (recommended):** name `flex-shrink: 0` + `max-width: 60%` of the header;
  summary takes the rest and ellipsizes first. Name ellipsizes only past its cap.
- **B (simplest):** name `flex-shrink: 1`, summary `flex-shrink: 100`. Name almost
  never shrinks in practice; no cap. Slightly less predictable at extremes.

Recommendation: **A** — bounded, predictable, the name always readable to ~60% of
the header before it too ellipsizes.

No TSX change is required for this part — it is purely the flex rules in
`block.scss`.

### 4.2 Remove the "continued" chip

Delete the chip and its supporting code end-to-end:

1. `agent-model.ts` — remove the chip block (`:150-162`), the
   `continuedFromMsAtom` field (`:74`), and `formatContinuationAgo` (`:25`) if it
   has no other caller.
2. `agent-view.tsx` — remove the `onContinuationModts` wiring (`:328`) and the
   nearby comment about the continuation chip (`~:324-328`).
3. `useHistoryPagination.ts` — remove the `onContinuationModts` option (`:78`) and
   its two invocation sites (`:321, :364`). **Verify first** that the `modts`
   value computed there serves no other consumer; if it does, keep the
   computation and only drop the callback.
4. CSS — the `agent-pane-continuation-chip` class has no dedicated rule (it rides
   `.block-frame-text`), so nothing to delete in `block.scss`; just stop emitting
   the className.

After removal, `viewText` returns only the `term:activity` summary (or `[]` when
there is none).

## 5. Out of scope

- The `term:activity` summary content/generation (Haiku-per-turn) is unchanged —
  only its layout priority changes.
- The widget-bar / other view types' headers (`blockframe.tsx` is shared) — the
  flex tweak is scoped to the agent header classes only; confirm no regression to
  term/editor/browser headers that reuse `.block-frame-textelems-wrapper`. If the
  wrapper rule is shared, scope the change with an agent-pane modifier class
  rather than editing the shared rule.

## 6. Test plan

- Narrow the agent pane: the **agent name stays fully visible** while the activity
  summary ellipsizes, then disappears; the name only ellipsizes past its cap
  (option A).
- A long agent name + long summary: name readable to its cap; summary degrades
  first.
- No "· continued …" chip ever appears; `tsc --noEmit` clean (no dangling refs to
  the removed atom/callback/formatter).
- Regression check: term / editor / browser pane headers still lay out correctly
  (shared `blockframe.tsx`).
