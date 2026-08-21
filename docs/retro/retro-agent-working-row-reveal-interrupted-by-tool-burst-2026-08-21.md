# Retro: Tool-call bursts interrupt the agent-pane "thinking" shimmer/type-out

**Date:** 2026-08-21
**Status:** Root cause confirmed via direct code reading, not yet fixed.
**Trigger:** operator report — "sometimes (not often) tool calls will
interrupt agent thinking dialog... I believe its on a state reducer" —
reproduced live in AgentY's pane.

## 1. Summary

The agent pane's footer "Working…" row (`AgentWorkingRow`,
`frontend/app/view/agent/components/AgentFooter.tsx`) type-out-reveals its
text character by character, then shimmers once fully revealed
(`docs/specs/SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md`).
When several tool-related events land in quick succession — parallel tool
calls in one assistant turn, or fast back-to-back tools — the row's text
resets to character 0 and restarts its reveal on **every** intermediate
transition instead of settling once, which reads visually as "the tool call
interrupted the thinking dialog." This matches the "sometimes, not often"
framing precisely: it only shows up during bursts, not on an ordinary
single-tool turn.

This was **not** found in the document/transcript content itself — the
actual thinking-block markdown is written and preserved correctly (see §3).
It is specifically a **presentation-layer race** in the footer's reveal
effect, one layer above where the operator's "state reducer" hunch pointed:
not `agent-document/reducer.ts` (which handles document nodes and is
already hardened against this class of bug, see §3), but
`agent-pane-state-store.ts`'s `dispatch()` — the pane-state store the
footer's props are read from.

## 2. Root cause — confirmed via direct code reading

1. **The event loop that fires tool transitions synchronously, unbatched:**
   `frontend/app/view/agent/useAgentStream.ts:540` — `for (const event of
   streamEvents) { ... }` iterates every `StreamEvent` translated from a
   single incoming backend chunk. Each `tool_call`/`tool_result` event in
   that loop independently calls `model.dispatchPane(...)` synchronously
   (lines 583–589 for `ToolStart`, line 592 for `ToolEnd`). A chunk carrying
   several tool transitions (parallel tool_use blocks, or several small
   tool calls arriving together) fires several `dispatchPane` calls in the
   same synchronous pass, back to back.

2. **No batching in the pane-state store's dispatch path:**
   `frontend/app/store/agent-pane-state-store.ts` — confirmed by direct
   grep: the file has **no `batch(` import or call anywhere**. Inside
   `dispatch()` (starting line 222), each changed field is applied via its
   own `proj(...)` call (e.g. `proj("currentTool", ...)` at line 337,
   `proj("currentToolArg", ...)` at line 351) — a raw SolidJS signal setter
   invoked directly, once per `dispatchPane` call. Every call in the
   unbatched loop from §2.1 lands here independently and fires its own
   reactive update immediately.

   For contrast: `useAgentStream.ts`'s own top doc comment (lines 17–35)
   documents that **document-node writes** (`ToolChunkAppend`,
   `StreamFlush`) were deliberately funneled through a shared
   `StreamFlushQueue` + RAF + `batch()` after a prior crash from unbatched
   writes. That discipline was never extended to `dispatchPane`'s
   `ToolStart`/`ToolEnd`/promotion path — this is the gap.

3. **The effect that pays for it, restarting on every flip:**
   `AgentFooter.tsx`:
   - `loadingLeftText()` (lines 113–128) returns the live tool name (e.g.
     `Bash · npm test`) instead of the rotating "Working…" phrase whenever
     `props.currentTool` is set and not yet `props.toolPromoted` (line 122).
   - `leftText` is a `createMemo` over that function (line 155) — it
     recomputes on every `currentTool`/`currentToolArg`/`toolPromoted`
     change.
   - The reveal effect (lines 181–202) reruns on **every** change to
     `leftText()`: unless `revealInstantly` is set, it calls
     `setRevealed(0)` and starts a fresh `setInterval` typing the string out
     at `REVEAL_CHAR_MS = 28`ms/char (lines 189–200).
   - `toolPromoted` flipping true a few seconds after a tool starts (the
     ActivityDock promotion) causes the **same** reset in reverse —
     `loadingLeftText` falls back from the tool name to the "Working…"
     phrase, again resetting `revealed` to 0.

**The race, concretely:** any burst of `ToolStart`/`ToolEnd` dispatches
processed in the same synchronous pass (§2.1) each independently flip
`currentTool`/`currentToolArg` (§2.2) with no batching, and each flip
independently re-triggers the reveal effect (§2.3) — restarting the
type-out from character 0 before the previous one finishes. Visually this
reads as the tool call interrupting the thinking-row text mid-reveal.

## 3. What this is NOT — ruled out

- **Not document/transcript corruption.** `frontend/app/view/agent/stream-parser.ts`
  accumulates thinking deltas into `currentThinkingNode` (`thinkingToNode`,
  lines 411–438), and each delta is flushed as its own document-node write
  via `queue.pushNewNode`/`pushUpdatedNode` before a following `tool_call`
  event resets the accumulator pointer (`eventToNode`, lines 340–343). No
  thinking content is lost or truncated in the actual transcript — this was
  confirmed by reading both `stream-parser.ts` and the reducer's own
  ordering guarantees (`agent-document/reducer.ts:507–524`, hardened
  against exactly this class of bug per
  `REPORT_AGENT_PANE_TEXT_TRUNCATION_2026-05-28.md`).
- **Not backend/wire-level reordering.** Anthropic's API delivers content
  blocks strictly sequentially (a thinking block's `content_block_stop`
  always precedes a following `tool_use` block's `content_block_start`).
  `agentmux-srv` passes these through without reordering
  (`agents/translator/claude.rs`). Confirmed directly against a live
  transcript pulled from AgentY's session (`GetAgentTranscript`,
  block_id `0a8d11f8-6962-486a-987e-2d4d366804da`) — the visible tail showed
  a single large sequential tool call (one `Write`, streamed as one
  `input_json_delta` block at a fixed content index) followed by a plain
  text reply, consistent with normal sequential turn structure.

## 4. Reproduction data

- Reported live in AgentY's pane (far-left pane, this host/channel).
- **Visual confirmation was not directly possible from this session**:
  `UIScreenshot`/`UIQuery` are scoped to "your own pane and shared app
  chrome" only (confirmed via tool description) — there is no cross-pane
  capture capability today (see `docs/analysis/ANALYSIS_AGENT_UI_AUTOMATION_CROSS_PANE_AND_CROSS_INSTANCE_TARGETING_2026_08_21.md`,
  written the same day, for exactly why that boundary exists and isn't a
  quick widening).
  `GetAgentTranscript` only returns text, and only the tail (server-capped
  at 500 lines) — the specific burst moment that triggered the operator's
  observation was not in the retrievable window, so this retro's root
  cause rests on direct code reading (§2), not a captured live burst.
- The mechanism reproduces deterministically from the code alone: any
  input that causes `useAgentStream.ts`'s per-chunk `streamEvents` loop
  (line 540) to contain more than one `tool_call`/`tool_result` transition
  — parallel tool use in one message being the clearest case — will fire
  more than one unbatched `dispatchPane` call in the same synchronous pass.

## 5. Recommended fix directions (not implemented here)

1. **Wrap the tool-transition dispatches in `batch()`** — either around the
   `for` loop in `useAgentStream.ts:540` (batch all `dispatchPane` calls
   produced from one incoming chunk), or inside `agent-pane-state-store.ts`'s
   `dispatch()` itself if it can detect/coalesce a rapid sequence of calls.
   The former is more surgical and matches the existing precedent
   (`StreamFlushQueue`'s RAF/batch discipline for document nodes) without
   touching the store's general dispatch path.
2. **Debounce/coalesce the reveal effect itself** — e.g. only restart the
   type-out if `leftText()` has been stable for some short window (a few
   ms), rather than reacting to every intermediate value in a burst. More
   defensive (guards against future unbatched writers too) but changes the
   footer's own timing model, not just the write side.
3. Either direction should add a regression test/story exercising a burst
   of 2+ `ToolStart`/`ToolEnd` dispatches in the same tick and asserting
   the reveal effect only restarts once (or not at all, if the net
   `currentTool` value is unchanged after the burst settles).

## 6. Sources

- `frontend/app/view/agent/useAgentStream.ts:540` (unbatched per-chunk
  event loop), `:581–592` (`ToolStart`/`ToolEnd` dispatch sites), `:17–35`
  (existing `StreamFlushQueue`/`batch()` precedent for document nodes)
- `frontend/app/store/agent-pane-state-store.ts:222` (`dispatch()`),
  `:337`, `:351` (`proj("currentTool"/"currentToolArg", ...)`) — no
  `batch(` anywhere in the file
- `frontend/app/view/agent/components/AgentFooter.tsx:113–128`
  (`loadingLeftText`), `:155` (`leftText` memo), `:181–202` (reveal effect)
- `frontend/app/view/agent/stream-parser.ts:340–343`, `:411–438`
  (thinking-node accumulation, ruled out as the cause)
- `docs/specs/SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md`
  (existing shimmer/type-out design)
- `docs/analysis/ANALYSIS_AGENT_UI_AUTOMATION_CROSS_PANE_AND_CROSS_INSTANCE_TARGETING_2026_08_21.md`
  (why cross-pane visual capture isn't available to confirm this live)
