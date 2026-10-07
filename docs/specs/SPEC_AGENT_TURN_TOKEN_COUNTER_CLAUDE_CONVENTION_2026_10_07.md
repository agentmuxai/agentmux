# Agent pane: one turn token counter, Claude Code's convention

**Status:** implemented — PR #4465, landed on main in PR #4468.
**Date:** 2026-10-07 · **Author:** agent2
**Component:** `AgentWorkingRow` (`frontend/app/view/agent/components/AgentFooter.tsx`)
**Token accounting:** `frontend/app/store/agent-pane-state/` (`reducer.ts`, `turn-contribution.ts`), `frontend/app/view/agent/main-agent-usage.ts`, `useAgentStream.ts`
**Replaces:** §3.4 ("Per-turn tokens: what the turn added") of `SPEC_AGENT_WORKING_ROW_MONO_SUMMARY_2026_10_02.md`. The rest of that spec stands.
**Builds on:** #4458. The live tokens now follow the stream's own order and survive the backend's turn-ended push, which reaches the pane before the turn's `session_end`.

---

## 1. Request, as given

> lets follow claude's convention, just one number with a toggling up/down. Those totals appear as they accumulate on the right. When it ends, the amount for just that turn appears in the "Worked · time · tokens"

Before this, the row showed two figures: `↑added ↓output` while working, and `✓ Worked · 42s · ↑added ↓output` when done. The `↑` figure was meant to be what the turn added to the context. Because of the race #4458 fixes, on Claude panes it showed every call's input summed (2.3m on a 135k context).

## 2. How Claude Code tallies its number

From Claude Code's own source. The npm release 2.0.30 still ships a readable `cli.js`; current 2.1.x builds are compiled binaries that carry the same identifiers (`addResponseLength`, `_responseLength`).

- **One number:** `KZ(Math.round(D / 4)) + " tokens"`. `D` eases toward `currentResponseLength`, the count of characters the model has streamed back. So it is an estimate of output tokens, at four characters a token.
- **What counts:** `rzA()` adds the length of every `text_delta`, `thinking_delta`, `input_json_delta` (tool-call input) and `signature_delta`. Input and context are never counted.
- **Monotonic per prompt:** the count resets to 0 only when the user submits a prompt (`uA(0)` on submit). It keeps adding across every API call and tool round of that prompt, and the displayed `D` only ever moves up toward it.
- **The arrow is the phase, not a second figure** (`eh6({mode})`):
  - `↑` for mode `requesting`: set by `stream_request_start`, when a request has been sent and nothing has streamed back yet. This happens again after every tool result.
  - `↓` for `responding`, `thinking`, `tool-input` and `tool-use`: the model is streaming, or its tool calls are running.

## 3. Design

### 3.1 While the turn runs (right zone)

`<arrow> <N> tokens · <elapsed>`, for example `↓ 2.4k tokens · 37s`.

- **N** is the turn's output so far, and only grows:
  - each finished call counts its exact `output_tokens` (from `message_delta`);
  - the call in progress counts the larger of its exact `output_tokens` so far and the characters it has streamed, divided by 4. That estimate makes the figure climb while text streams, as Claude Code's does; the exact count replaces it when `message_delta` arrives.
  - A high-water mark keeps the shown figure from ever going down, for example when a call's exact count comes in below its estimate.
- **Arrow:**
  - `↑` from a main-agent tool result until the next call's `message_start` (a request is in flight).
  - `↓` from `message_start` on: the model streams, then its tools run.
  - The first request of the turn shows no figure yet. The segment appears with the first `message_start`, so the row never shows `↑ 0 tokens`.
- **Main agent only.** Subagent lines (`parent_tool_use_id` set) are ignored, as they already are for the context meter (`main-agent-usage.ts`). Claude Code adds subagent text into its spinner. We don't, so the live figure and the finished figure (the `result` frame's usage, which is the main agent's) count the same thing.
- **Providers without live usage** (everything except the `claude-stream-json` format: Codex, Gemini, Qwen, Kimi, Copilot, the ACP panes) show the elapsed time only, as before.

### 3.2 When the turn ends (left zone)

`✓ Worked · <time> · <N> tokens`, for example `✓ Worked · 1m 4s · 2.4k tokens`.

- **N** is the `result` frame's `output_tokens`: the CLI's exact output for the whole turn, summed over its calls. It can differ slightly from the last live figure, which was partly an estimate.
- With no `output_tokens` in the result, N is the live figure from `turnTokens`, which #4458 keeps until the turn's own `TurnEnd`.
- With neither, the segment is left out.
- There is no arrow: nothing is moving any more.
- The right zone (`$cost · K turns`) is unchanged.

### 3.3 What goes

- **The `↑` input figure, live and finished.** The context-window meter in the composer already shows how big the context is, and the per-turn "added" figure is no longer shown anywhere.
- **The code that only fed it:** `turnAddedInput()`, `TurnTokens.contextBaseline` and `SessionStats.added_input_tokens`.

## 4. Implementation

- **`main-agent-usage.ts`**, two readers next to `mainAgentUsage`, main agent only:
  - `mainAgentStreamedChars(line)`: the characters a `content_block_delta` adds (`text`, `thinking`, `partial_json`). `signature_delta` isn't counted: it is a signature, not model output.
  - `mainAgentRequestStarted(line)`: true for a `user` frame that carries a `tool_result` block, meaning the CLI is sending the next request.
- **`useAgentStream.ts`:**
  - Sums the streamed characters over each batch of lines and dispatches one `OutputStreamed { chars }` per batch, not one per delta.
  - Dispatches `RequestStarted` on a main-agent tool result.
  - Both only when the stream carries Claude Code's usage (`readsMainAgentUsage`).
- **Reducer** (`TurnTokens` gains `outputDone`, `streamedChars`, `requesting`, `shownOutput`):
  - `TokensIn` (a new call, the stream already dedups repeats by message id): folds the previous call's output into `outputDone`, then resets `output` and `streamedChars`, and sets `requesting: false`.
  - `TokensOut`: sets the current call's exact `output`.
  - `OutputStreamed`: adds to `streamedChars`.
  - `RequestStarted`: sets `requesting: true`.
  - All four raise `shownOutput` to the new total, never lower it.
  - `OutputStreamed` and `RequestStarted` do nothing before the turn's first `message_start` (no `turnTokens` yet).
- **`turn-contribution.ts`:** `turnAddedInput` is replaced by `turnOutputTokens(t)`, the shown live figure.
- **`mergeStats`:** `output_tokens` is the result's, falling back to `turnOutputTokens` of the live tokens.
- **`useTurnLifecycle.ts`:** the status-bar totals fallback, used when a result has no usage, records the turn's total output from the live tokens, not the last call's.
- **`AgentFooter.tsx`:** the formatting in 3.1 and 3.2.

## 5. Tests

- **Reducer:**
  - output adds up across calls;
  - streamed characters show as an estimate until the exact count arrives;
  - the shown figure never drops when the exact count is lower than the estimate;
  - the arrow goes `↓` on `message_start` and `↑` on a tool result;
  - nothing happens before the first `message_start`;
  - `TurnEnd` keeps the result's `output_tokens`, and falls back to the live total.
- **`main-agent-usage`:** both readers ignore subagent lines and count only what section 4 says.
- **`AgentWorkingRow`:**
  - the live right zone reads `↓ 2.4k tokens` (and `↑ …` after a tool result), then the elapsed time;
  - the finished row reads `✓ Worked · 42s · 2.4k tokens`, with no `↑` input figure.
