# Agent pane "Working…" row: composer typeface, pane-border color, ambient summary, per-turn tokens that are the turn's own

**Status:** implemented (spec and change together).
**Date:** 2026-10-02
**Owner:** Manoz
**Component:** `AgentWorkingRow` (`frontend/app/view/agent/components/AgentFooter.tsx`)
**Styles:** `.agent-working-row-anchor .agent-working-row--loading` (`frontend/app/view/agent/styles/_control-bar.scss`)
**Token accounting:** `frontend/app/store/agent-pane-state/turn-contribution.ts`, `reducer.ts` (`TokensIn`, `TokensOut`, `mergeStats`)
**Supersedes in part:** `SPEC_AGENT_WORKING_ROW_TYPOGRAPHY_REFRESH_2026_09_03.md` (typeface and color) and the shimmer half of `SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md` §2.

## 1. Request, as given

> lets handle the "Working..." animation, part of the progress system in the agent pane. It should have the same font as the user input, which is a monospace type font. Also, get rid of the token count at the far right, since it already appears in the composer.

> it should also have the same color ... we also dont want the tool call, instead, Id like to integrate ambient haiku there

> but those numbers are totals, not per turn, so they need to be fixed

> ah, I see...per turn is essentially total, since the context is sent back every time. For that number, we dont want the total context sent every turn, we want the diff added

> keep the per-turn, but make it the actual contribution of that turn (not the total to the context)

How the requests were resolved:

- "Ambient haiku" means the **ambient activity summary** (the Haiku-model text the app already generates), not a new generated poem.
- "Same color" means **the pane's border color**, as the agent pane shows it: the agent's color, with the theme accent as fallback. The faint unfocused-border grey was rejected as unreadable at 10px.
- The shimmer sweep goes (see 3.2).
- The token count was first to be removed on the belief that the composer already shows it. It does not (see 4), so it stays, with a corrected definition (3.4).

## 2. What the row looked like

`AgentWorkingRow` in its `--loading` state: a pulsing dot, a left zone, and a right zone.

| Part | Before |
|---|---|
| Typeface | `--markdown-font-family`, the thinking-text face (chosen 2026-09-03 on the grounds that a status sentence "isn't code-shaped") |
| Color | `--main-text-color`, with a gradient shimmer sweeping between `--secondary-text-color` (dim, most of the time) and `--main-text-color` |
| Left zone | The cycling "Working…" phrase, or `tool · arg` (for example `Read · …/AgentFooter.tsx`) whenever a tool call was running and not yet promoted to an ActivityDock row |
| Right zone | `↑in ↓out` turn tokens, then the elapsed time. The `↑` figure was the size of the whole conversation, re-sent on every call |

## 3. Change

### 3.1 Typeface

`font-family: var(--font-mono)` on `--loading`. This is the composer's face: `.agent-input` declares `font-family: inherit`, and the nearest declaration above it is `.agent-view { font-family: var(--font-mono) }` (`agent-view.scss`). Naming the same variable keeps the two identical if the theme's mono face changes. Size (10px) and weight (700) are unchanged. `--worked` is untouched.

### 3.2 Color, and the shimmer

`color: var(--accent-color)`, overridden by `.has-agent-color & { color: var(--block-agent-color) }`.

- `.has-agent-color` is set on the block frame (`blockframe.tsx`) exactly when the pane has an agent color, and the same condition picks `--block-agent-color` over the accent for the focused border (`block.scss`).
- It cannot be a `var()` fallback: `blockframe.tsx` always sets `--block-agent-color`, to `"transparent"` when there is none, so a fallback would never apply and the text would vanish.
- The text is now one solid color. The gradient sweep, its keyframes and the `.is-typing` overlay (which existed only to cover the sweep during the reveal) are removed, along with the `typing` signal. A sweep between two stops of the same color would be invisible, and the dim half of the old sweep is what the request objected to. The pulsing dot beside the text still signals liveness, and the character-by-character type-out reveal of new text is kept.

### 3.3 Left zone: ambient summary instead of the tool call

Priority, first match wins:

1. Reconnecting, Compacting, Stopping, Rate limited, or a launch phase (unchanged).
2. **The ambient summary**, `readSwarmSummary(block.meta)`: the same line the Swarm row and pane tooltip show. That is `term:ambient_summary` (Haiku-written, validated by `isUsableTitle`), falling back to the CLI's own `term:osc_title` topic.
3. The cycling "Working…" phrase, until a summary exists.

What this text actually is: `useAgentActivitySummary.ts` keeps a stable, PR-title-style phrase for the session's **overall goal**, re-evaluated when the user submits a message. It is not a per-tool-call description. So the row reads, for example, "Fix the login redirect loop" for the whole turn rather than changing with every tool. A per-activity ambient call would be a new feature (prompt, rate limit and token cost in `crates/srv/src/ambient/`); it is out of scope here.

The pane header stopped showing this summary on 2026-09-20 (`agent-model.ts`, "pane-color unification"). This change surfaces it in the working row instead; the Swarm row is unaffected.

Consequences of dropping the tool text:

- `currentTool`, `currentToolArg` and `toolPromoted` are removed from `AgentWorkingRow` and its call site, along with the local `abbreviateArg` helper.
- The phrase-cycling effects no longer pause during a tool call, because nothing tool-specific is on screen to protect.
- The `hasPromotedTool` chain is removed end to end, since its only reader was the tool text: the prop into `AgentBottomPanels`, the memo and the `promotionTick` option in `useWorkingIndicator`, and `hasRunningPromotedTool` (with its tests) in `tool-adapter.ts`. `agent-view.tsx` keeps the promotion clock for `useAttachedTaskAxis`.
- The live tool call remains visible where it always had a better home: the ActivityDock rows and the document view.

### 3.4 Per-turn tokens: what the turn added, not the context it re-sent

**The problem.** Every API call in a turn re-sends the whole conversation. So the `↑` figure was never "this turn": the live value was the input of the *last call* (`TokensIn` overwrites), which is the size of the context; and the "Worked" value was the result event's whole-turn `input_tokens`, which sums that context over *every* call (the reducer's own test fixture has a 70,000 result total for a turn whose last call was 2). A one-line question late in a long session read as `↑180k`. The composer's `123k / 200k` is the same kind of number on purpose: it is the context-window meter.

**The definition.** The turn's input contribution is the growth of the context across the turn:

```
added = context size at the turn's last call  -  context size before the turn began
```

That counts the user's message, the tool results, and the assistant's earlier output that became input to the next call, and excludes everything that was merely re-sent. It needs no cache breakdown, so it works for any provider that reports a per-call input size.

- **Baseline.** `TokensIn` stores `contextBaseline` on `turnTokens` at the turn's first call and carries it unchanged afterward (`TokensOut` preserves it). The value is `lastContextTokens` as it stood before the turn (the previous turn's last context size, which the resume path also seeds). When there is none (a fresh pane, or after `TurnReset`) the first call's own input stands in, so the turn is not credited with the system prompt.
- **Never negative.** A compaction inside a turn shrinks the context; that reads as `0`, not a negative contribution.
- **Live.** The right zone shows `↑added ↓output` and the elapsed time, computed by `turnAddedInput()` from `turnTokens`. Where a provider reports no live usage there is no baseline, and the readout falls back to the raw input rather than inventing a figure.
- **Finished.** `mergeStats` stores `added_input_tokens` on `sessionStats` at `TurnEnd`, and the "✓ Worked · 42s · ↑… ↓…" line shows it (falling back to `input_tokens` when absent). So the live row and the completion line use the same definition.
- **Output** is already a per-turn quantity and is shown as reported. (Live, it is the current message's running output; the completion line uses the result's whole-turn figure. That difference predates this change.)

**Deliberately unchanged:** `input_tokens` on `sessionStats`, `sessionTotals` and the token-usage store keep the raw, re-sent numbers. Those feed the session-stats popover and cost/context accounting, where "what was sent" is the point. Only the *display of one turn* changed.

## 4. The premise that turned out wrong

The request said tokens "already appear in the composer". What the composer strip shows is **context-window usage** (`123k / 200k`, with an auto-compact warning band), a cumulative figure. Its own per-turn stats readout was dropped on 2026-08-31 for duplicating this row. So removing the count from the row would have left no per-turn figure at all, and the per-turn figure was itself mis-defined (3.4). The decision after seeing that: keep it in the row, and make it the turn's real contribution.

## 5. Not changed

- Row layout, size, padding and the opaque background.
- The `--worked` completion row's typography.
- The pulsing dot, the type-out reveal, and the reduced-motion handling of the reveal.
- The ambient summary's generation (`useAgentActivitySummary.ts`) and the Swarm row.
- Context-window meter, `sessionTotals`, and the session-stats popover.

## 6. Tests

`reducer.test.ts`, "Tokens: what the turn added, not the context it re-sent":

- the baseline is the previous turn's last context size;
- the baseline survives the turn's later calls while `lastContextTokens` moves on;
- with no earlier context the first call stands in (contribution 0);
- a shrinking context is never negative;
- no baseline gives `undefined`;
- `TurnEnd` records `added_input_tokens` next to the re-sent result total;
- `sessionTotals` still sums the raw input.

`AgentFooter.test.tsx`, "AgentWorkingRow ambient summary and per-turn tokens":

- the trimmed summary shows in the left zone, with the phrase as fallback, and a status wins;
- the right zone shows the contribution (`↑140k`, not the `180k` re-sent) before the elapsed time, and falls back to raw input without a baseline;
- the Worked line shows `added_input_tokens`, not the summed `840k`;
- the left zone carries no shimmer or typing classes.

The typeface and color are CSS-only and were checked by compiling `agent-view.scss` and reading the emitted rules: `.agent-working-row--loading` gets `font-family: var(--font-mono)` and `color: var(--accent-color)`, the `.has-agent-color …--loading` rule overrides the color, and no `shimmer` selector remains.
