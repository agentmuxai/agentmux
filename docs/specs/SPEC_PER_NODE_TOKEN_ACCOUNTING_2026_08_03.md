# Spec: true per-node token accounting

**Status:** proposed — verified unimplemented as of 2026-08-10 (no roundIndex/RoundRecord in frontend); base hover-peek shipped separately (PR #2392).
**Relationship:** Phase 2 follow-up to `docs/specs/SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md`, which deliberately deferred real per-node token/cost data (§1.4/§4.2/§6 there) in favor of a client-side chars÷4 estimate. This spec answers "can we do better than a guess?" — and the answer is **partially yes, with a real derivation**, not a heuristic, for the Claude provider specifically.

## 1. The core insight

`message_start`'s `usage.input_tokens` (`claude-translator.ts`, intercepted in `useAgentStream.ts:439-473`) is not a delta — it's the **cumulative prompt size for the whole conversation at the moment that API round began**. Claude Code makes one API round per `message_start`…`message_stop` cycle, and — in persistent/multi-tool-round mode — **a fresh `message_start` fires after every `tool_result`**, because each continuation round is its own API request with the prior turn's tool result now appended to context.

That means consecutive rounds' `input_tokens` values bound a real, computable quantity: **how much the context grew between the start of round N and the start of round N+1** — which is exactly "round N's own output (thinking + text + tool_use encoding) plus whatever tool result got appended after it finished." Since round N's own output token count is *also* independently known (`message_delta`'s `usage.output_tokens`, which arrives once per round, right before `message_stop`), the tool-result's token cost can be isolated by subtraction:

```
toolResultTokens(N) = inputTokens(N+1) − inputTokens(N) − outputTokens(N)
```

Both operands are real numbers straight off the wire — not an estimate of content length, an actual accounting identity. This is the number worth surfacing on a tool-call hover as "Result: N tokens," unqualified (no "~", no "(est.)"), when the assumptions in §3 hold.

## 2. What's real vs. what remains a gap

| Quantity | Source | Real or estimate? |
|---|---|---|
| Round N's own generation tokens (thinking + text + tool_use overhead) | `message_delta`'s `usage.output_tokens` for round N | **Real** — direct from the wire, once per round |
| Context growth from round N → N+1 | `inputTokens(N+1) − inputTokens(N)` | **Real** — both operands are wire values |
| Tool N's result cost | `contextGrowth(N→N+1) − outputTokens(N)` | **Real, derived** — exact *if* round N had exactly one tool call and nothing else perturbed context size in between (§3) |
| Split of a round's output tokens across *multiple* nodes created in that same round (e.g. a thinking clump immediately followed by a tool_use, both in round N) | — | **Not derivable.** The wire only reports one total per round; there's no sub-round signal. See §3.1. |
| Anything for Codex, Gemini, or other non-Claude providers | — | **Not available.** See §3.2. |

## 3. Limits — where the derivation breaks down

### 3.1 Multiple nodes in one round

A single round commonly produces more than one `DocumentNode` — e.g. a thinking clump followed by a tool call, both under one `message_start`/`message_delta` pair. The round's one `output_tokens` figure covers all of it; there is no wire signal to divide "how much was thinking vs. how much was the tool_use JSON encoding." **This spec does not attempt to split it.** Instead: if a round produced exactly one node, that node gets the round's real output-token figure, unqualified. If a round produced more than one node, every node in that round shows the same round-level figure, explicitly labeled as shared (e.g. "Generation this round: 340 tokens (shared with 1 other block)") — honest about the granularity rather than fabricating a split.

### 3.2 Compaction interference

`inputTokens(N+1) − inputTokens(N)` assumes nothing else changed context size between the two rounds. A real `CompactionBoundary` (or the heuristic ≥50%-drop fallback — see `SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`) landing in that window invalidates the whole diff — a compaction shrinks context in a way indistinguishable, by the numbers alone, from "a huge tool result got appended." **Any round pairing that straddles a compaction event (checked via the reducer's existing `lastCompactionBoundaryAt`/`compacting` state) must be treated as unavailable, not computed and silently wrong.**

### 3.3 Parallel tool calls in one round

If a round contains more than one tool call (parallel tool use), `toolResultTokens(N)` is the combined cost of all of them — there's no way to attribute the aggregate to one call over another. Same treatment as §3.1: show the shared aggregate on every tool call in that round, labeled as shared, rather than guessing a split.

### 3.4 Provider coverage — Claude only

Checked all three translators:
- **Claude** (`claude-translator.ts`): per-round `message_start`/`message_delta`, as described above.
- **Codex** (`codex-translator.ts:59-65`): usage arrives exactly once, on `turn.completed` (`total_usage`/`usage`), covering the **entire multi-round turn** — no per-round breakdown exists in Codex's own wire format.
- **Gemini** (`gemini-translator.ts:54-55`): same shape — one stats object, turn-final, not per-round.

**This feature is Claude-only.** Codex/Gemini/other providers have no sub-turn granularity to derive from — there is nothing in their CLI output for AgentMux to read that would enable this, short of asking each provider's team to add per-round usage reporting (out of scope; a provider-side ask, not an AgentMux change). For those providers, tool-call/thinking-clump hovers fall back to either the chars÷4 estimate from the hover-peek spec (clearly labeled) or no token figure at all (§6 Q2).

### 3.5 Live-session only, not history replay

Checked `parseHistoryLines.ts` (lines ~21-151): history replay only reconstructs turn-level aggregates from `session_end`'s `stats` payload — the raw per-round `message_start`/`message_delta` frames this derivation needs are never persisted or replayed. **This feature only works for turns that happen while the pane is actively open and streaming in the current session** — reopening a pane and hovering an old tool call from before the reopen will show no per-node figure, same limitation the (unshipped) `node-timestamp-hover.md` spec already accepted for its own timestamps.

## 4. Design

### 4.1 Round tracking — new, local bookkeeping in `useAgentStream.ts`

Not a reducer concern — this is derived, display-only data, not correctness-critical state, so it should NOT grow `AgentPaneState`/the reducer's surface. Keep it as local, non-reactive state in the `useAgentStream` hook's closure, scoped to the pane's lifetime:

```ts
interface RoundRecord {
    roundIndex: number;
    inputTokensAtStart: number;
    outputTokensAtEnd: number | null; // null until this round's message_delta lands
    straddledCompaction: boolean;     // set true if a CompactionBoundary/heuristic fired during this round
}
```

- A `roundIndex` counter increments on every `message_start` seen (alongside the existing `TokensIn` dispatch, `useAgentStream.ts:444-451`).
- Each `RoundRecord` is stored in a small `Map<number, RoundRecord>` (or array), cleared on `TurnEnd`/`TurnReset`/pane unmount — mirrors `turnTokens`'s own lifetime, no unbounded growth.
- On `message_delta` (`useAgentStream.ts:468-473`), fill in `outputTokensAtEnd` for the *current* round before the counter advances.
- On any compaction signal (`CompactionStarted`/`CompactionBoundary` pane events, or the existing heuristic path), mark the in-flight round `straddledCompaction: true`.

### 4.2 Tagging nodes with their round

Add `roundIndex?: number` to `ToolNode` and `MarkdownNode` (thinking-flagged) in `types.ts` — stamped at creation time in `stream-parser.ts` (`toolCallToNode()`/`thinkingToNode()`), read from the same counter `useAgentStream.ts` maintains (threaded through as a parameter, same pattern `now: number = Date.now()` already uses for testability).

### 4.3 Deriving and attaching `toolResultTokens`

When round N+1's `message_start` arrives (i.e., round N is now fully closed):
```ts
if (roundN.outputTokensAtEnd != null && !roundN.straddledCompaction && !roundNPlus1.straddledCompaction) {
    const derived = roundNPlus1.inputTokensAtStart - roundN.inputTokensAtStart - roundN.outputTokensAtEnd;
    if (derived >= 0) attachToolResultTokens(roundN.roundIndex, derived);
    // negative => an assumption broke silently; treat as unavailable, do not display a nonsense figure
}
```
Attach the result to whichever `ToolNode`(s) carry `roundIndex === roundN.roundIndex` (commonly exactly one — §3.1/§3.3 cover the multi-node/multi-call case).

### 4.4 Rendering — extends the hover-peek spec's tooltip content, doesn't replace it

Builds directly on `SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md` §2.3/§2.4's tooltip `content=`:

- **Tool call**, when real data is available: replace the chars÷4 estimate line with `Result: N tokens` (unqualified — it's real) plus, if the round had other nodes, `Generation this round: N tokens (shared with K other blocks)`. When real data ISN'T available (non-Claude, compaction-straddled, negative/broken diff, or history-replayed node): fall back to the estimate line from the base spec, or omit (§6 Q2).
- **Thinking clump**: if it's the round's only node, `N tokens` (real, unqualified). If shared with a tool call in the same round, same shared-aggregate wording as above.

## 5. Files touched

| File | Change |
|---|---|
| `frontend/app/view/agent/types.ts` | Add `roundIndex?: number` to `ToolNode`, `MarkdownNode` |
| `frontend/app/view/agent/stream-parser.ts` | Accept/stamp `roundIndex` in `toolCallToNode()`/`thinkingToNode()` |
| `frontend/app/view/agent/useAgentStream.ts` | New local round-tracking map (§4.1); derive + attach `toolResultTokens` on each new `message_start` (§4.3); pass current `roundIndex` into node-creation calls |
| `frontend/app/view/agent/components/ToolBlock.tsx` | Tooltip content: prefer real `Result:`/`Generation this round:` figures over the base spec's estimate when available |
| `frontend/app/view/agent/components/MarkdownBlock.tsx` | Same preference for thinking-clump tooltip |

## 6. Open questions — resolved

1. **Multi-tool-call rounds (§3.3)** — **resolved: show the shared aggregate with the "(shared)" caveat**, not suppressed. Consistency beats suppression: §3.1 already shows a shared figure (with caveat) for the mixed thinking+tool-call-in-one-round case, so parallel tool calls should follow the same one rule rather than a second, different behavior for a structurally similar situation. Suppressing entirely would also hide the exact case — a burst of parallel tool calls — where a user most wants a cost signal, even an aggregate one.
2. **No-data fallback** — **resolved: fall back to the chars÷4 estimate** (labeled "(est.)"), not silence. This is the same decision as `SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md` §4 resolution 2, answered once and shared by both specs: the estimate ships as the v1 default there, and here it doubles as the fallback whenever real accounting isn't derivable (non-Claude, compaction-straddled, history-replayed). One mechanism, one place it's labeled, consistent behavior whether or not real data happens to be available for a given node.
3. **Worth the complexity?** — **resolved: sequence it.** Implement `SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md` (hover UI + chars÷4 estimate) first, as its own complete, shippable pass — no new pane-level bookkeeping, small diff. This spec's round-tracking/derivation lands as a second, later pass that *upgrades* the same tooltip surface for Claude panes when real data is available, falling back to the already-shipped estimate everywhere else. Not built simultaneously with the base spec.

## 7. Verification plan (once implemented)

- Live session, Claude provider, a turn with exactly one tool call per round: hovering the tool shows an unqualified "Result: N tokens" that sanity-checks against the visible context-fill jump in the composer strip.
- A round with thinking-then-tool-call together: both the thinking-clump hover and the tool-call hover show the same "Generation this round: N tokens (shared with 1 other block)" figure.
- Trigger a compaction mid-turn: the round straddling it shows no derived figure (falls back per §6 Q2), not a wildly wrong number.
- Switch to a Codex/Gemini pane: no per-node figures anywhere (or the base spec's estimate, per §6 Q2) — confirms the Claude-only gate.
- Reopen a pane from history and hover an old tool call: no per-node figure (§3.5) — confirms this doesn't silently show stale/wrong data for replayed nodes.

## 8. Out of scope

- Splitting a round's output tokens across multiple same-round nodes by any heuristic (content-length proportional or otherwise) — considered and rejected in favor of the honest "shared" label (§3.1). Could be revisited as an explicit, clearly-labeled-as-guessed enhancement later if users want a rough split badly enough.
- Persisting round-level token data into history so replayed panes get real figures too — would need backend/persistence changes (storing raw usage frames, not just the final `stats` snapshot); a separate, larger effort if ever wanted.
- Extending real accounting to Codex/Gemini — blocked on those providers' own CLI output format, not something AgentMux can derive from what's available today (§3.4).
