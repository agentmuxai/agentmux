# Spec: Token stats panel — break out by agent + value-add details

**Date:** 2026-08-30
**Status:** Proposed
**Motivated by:** direct request — the status bar's token breakdown popover
currently groups by provider/service only; break it out by agent instead,
and identify what else is worth adding while touching this panel.

## Background

The status bar's `TokenUsageIndicator` (`frontend/app/statusbar/`) shows a
running `↑X ↓Y` total for the whole AgentMux session and opens
`TokenBreakdownPopover` on click. The popover is driven by
`frontend/app/store/token-usage.ts`, a session-local store keyed by
**provider id** (`"claude"`, `"codex"`, `"gemini"`, …) — see
`SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md`. `InstancePanel.tsx`'s own doc
comment already flagged this as a known gap: *"Per-window token totals
deferred until token-usage is per-window."* This spec is that follow-up.

Two things make "by agent" more than a regroup:

1. **`recordTurn(provider, tokens)` has no agent identity at all today.**
   Its real call site, `useTurnLifecycle.ts:110`, has `opts.blockId` and
   `opts.model` (`AgentPaneModel`) in scope but doesn't pass either through
   — grouping by agent requires resolving and threading agent identity
   into the store for the first time, not just re-reading existing data.
2. **`provider` is already overloaded with non-agent pseudo-ids.**
   `ActivityDock.tsx`, `useNextPromptSuggestion.ts`,
   `useAgentActivitySummary.ts`, and `swarm-view.tsx` all call `recordTurn`
   with ids like `"ambient:next_prompt_suggestion"`,
   `"ambient:activity_summary"`, `"ambient:subagent_name"` — real token
   spend from AgentMux's own background features, not a user's agent. The
   current breakdown lists these as peer rows next to `"claude"` with no
   visual distinction, which is confusing and gets worse once real agent
   rows are also broken out individually.

Separately, `SessionStats` (`frontend/app/view/agent/types.ts:545`) already
carries `cost_usd`, `duration_ms`, and `num_turns` **per pane**, accumulated
cumulatively in `AgentPaneState.sessionTotals`
(`SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md`) — and none of it reaches
the status bar today. `AgentFooter.tsx:281` already has the display
convention (`` `$${cost_usd.toFixed(3)}` ``) to reuse.

## What's worth adding, beyond "group by agent"

Ranked by value vs. cost, given what's already tracked somewhere in the
codebase (cheap to surface) vs. what needs new plumbing:

1. **Cost in USD, per agent and total.** `cost_usd` already exists per pane
   and is never shown outside that one pane's own composer strip. This is
   the single highest-value addition — token counts alone don't answer
   "what is this session costing me," which is the question a user
   opening this popover is most likely asking. Reuse the `$X.XXX` format
   from `AgentFooter.tsx`.
2. **Turn count per agent.** `num_turns` is already tracked alongside
   `cost_usd` in the same `SessionStats` — free once cost is threaded
   through. Read together with tokens it gives a cheap tokens/turn signal
   (an agent burning 50k tokens over 3 turns reads very differently from
   50k over 30).
3. **Separate "Agents" from "AgentMux internal."** The ambient pseudo-ids
   above are real spend but not a user's agent — collapse them into one
   "AgentMux internal" row (background suggestions, activity summaries,
   subagent naming) instead of interleaving with per-agent rows. Directly
   fixes the confusion described above, and costs nothing new to compute —
   it's a matter of not losing the `ambient:` prefix that already exists in
   every one of these ids, which the current per-service view discards by
   treating it as just another service name.
4. **Click a row to focus that agent's pane.** The by-agent grouping
   naturally carries a `blockId` per row (Instance Panel's window list
   already does exactly this — focus-on-click for a `blockId`, see
   `InstancePanel.tsx`). Low incremental cost once rows are keyed by agent,
   turns a read-only breakdown into a navigation aid ("which agent is
   burning tokens" → click → look at it).
5. **Per-agent cache hit rate.** `getCacheHitRate()` already exists
   globally (`token-usage.ts:133`); the same computation applies per-agent
   once usage is bucketed that way. Surfaces which agents are cache-
   efficient (stable, append-only context) vs. not (context that churns
   every turn, e.g. frequent file-content injection) — useful for a user
   trying to reduce spend. Lower priority than 1–3: a secondary metric, not
   the first thing someone opening this panel wants.
6. **Active vs. idle/retired grouping.** AgentMux's swarm view already
   distinguishes active from retired agents. Splitting the panel the same
   way (currently-running agents first, finished ones collapsed below)
   keeps a long swarm session's breakdown scannable instead of a flat list
   growing forever. Worth doing once agent count in a session is
   realistically > 5–6; lower priority for the initial cut.
7. **Session burn rate (cost or tokens per minute).** Cheap to derive
   (`total / (now - sessionStartAt)`) but of debatable value — a single
   number that's noisy early in a session and mostly redundant with the
   total once you can already see elapsed time via "since HH:MM" in the
   header. Listed for completeness; **not recommended for v1** (see
   Non-goals).

## Design

### Store: `frontend/app/store/token-usage.ts`

Add agent-keyed aggregation alongside the existing service-keyed one
(kept, not replaced — some ambient/ancillary callers have no agent
identity to attach, and per-service is still meaningful inside an agent
row when a pane forks across providers).

```ts
export interface AgentUsage {
    agentName: string;       // display name, from block.meta.agentName
    blockId: string | null;  // null only for the "AgentMux internal" bucket
    isAmbient: boolean;      // true for the internal bucket
    input: number;
    output: number;
    costUsd: number;         // 0 if provider never reports cost_usd
    numTurns: number;
    freshInput?: number;
    cacheCreation?: number;
    cacheRead?: number;
    byService: Record<string, ServiceUsage>; // existing per-service shape, nested
}
```

- New `byAgent: Record<string, AgentUsage>` field on `TokenUsageState`,
  keyed by `blockId` for real agents and by a fixed sentinel key (e.g.
  `"__ambient__"`) for the internal bucket.
- `recordTurn` gains an optional third parameter:
  ```ts
  export function recordTurn(
      provider: string,
      tokens: ServiceUsage | null | undefined,
      agent?: { blockId: string; agentName: string; costUsd?: number },
  ): void
  ```
  Omitting `agent` (all four existing ambient call sites) files the turn
  under the `"__ambient__"` bucket, tagged `isAmbient: true` — no call-site
  change required at those four sites beyond leaving the third arg off,
  so the "Agents vs. internal" split (item 3) falls out of the *existing*
  `ambient:`-prefixed provider ids for free, just read at the right level
  instead of discarded.
- `getAgentBreakdown()`: sibling to `getBreakdown()`, returns `AgentUsage[]`
  sorted by `costUsd` descending (falls back to token total when
  `costUsd` is 0/unknown for every row — some providers may never report
  cost), real agents before the ambient bucket regardless of its size.
- `getAgentCacheHitRate(row: AgentUsage)`: same formula as the existing
  global `getCacheHitRate()`, applied to one `AgentUsage` row.

### Call-site change: `frontend/app/view/agent/hooks/useTurnLifecycle.ts`

The only call site with genuine per-agent turn data. At the existing
`recordTurn(opts.provider, tokens)` call (line 110):

- Resolve `agentName` from `block.meta.agentName` via
  `WOS.getWaveObjectAtom<Block>(makeORef("block", opts.blockId))()` — same
  pattern `armory-model.ts` already uses to read block meta reactively;
  here it's a one-shot read at turn-finalize time, not a reactive memo.
  Fall back to the raw `blockId` (truncated, matching the existing
  `blockId.slice(0, 7)` convention already used for log lines in this same
  file) if `agentName` is unset.
- Pass `stats?.cost_usd` through as `costUsd` (the `stats` merge already
  in scope for `statsTokens` above).
- The other three call sites (`ActivityDock.tsx`, `useNextPromptSuggestion.ts`,
  `useAgentActivitySummary.ts`) and `swarm-view.tsx`'s `"ambient:subagent_name"`
  call stay unchanged — they have no pane-level `SessionStats` to draw
  `agentName`/`costUsd` from, and per the design above that's fine: they
  fall into the ambient bucket automatically.

### UI: `TokenBreakdownPopover.tsx`

- Rows become per-agent instead of per-service: agent name, `↑`/`↓`
  tokens, turn count, `$cost` (value-adds 1–2). Omit the `$` segment
  entirely for a row whose `costUsd` is 0 across every turn (provider
  never reported cost) rather than showing a misleading `$0.000`.
- Clicking a real-agent row (not the ambient one) focuses that pane —
  reuse whatever pane-focus action `InstancePanel.tsx`'s window list
  already calls for a `blockId` (item 4).
- "AgentMux internal" renders as a single collapsed row at the bottom,
  expandable to the existing per-service view for just that bucket (item
  3) — collapsed by default so it doesn't compete visually with real
  agent rows.
- Per-agent cache hit rate (item 5) as a small inline `%` next to a row,
  same `title=` tooltip-on-hover treatment the existing global cache-rate
  line uses — not a v1 requirement, can land after the base regroup.
- Grand total row and "Reset counter" footer stay as-is (still a global
  reset — resetting per-agent is a non-goal, see below).

## Non-goals

- **No cross-restart persistence.** Store stays session-local, same as
  today (`token-usage.ts`'s own doc comment already flags this as a
  separate stretch goal in the original spec) — out of scope here.
- **No per-turn history/timeline graph.** This spec only extends the
  existing cumulative-since-session-start model to a new grouping axis; a
  time-series view is a materially bigger feature.
- **No session burn-rate metric (item 7)** in the initial cut — low value
  relative to the other items, revisit only if requested after the
  by-agent regroup ships.
- **No per-agent reset.** "Reset counter" stays a single global action;
  scoping it to one agent row adds UI complexity (confirm-per-row) for a
  use case not raised in the motivating request.
- **No backend changes.** Every field this spec adds (`cost_usd`,
  `blockId`, `agentName`) is already available frontend-side today —
  this is purely a frontend aggregation + display change, same framing as
  `SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md`'s own non-goals.
- **Active/idle grouping (item 6)** deferred to a follow-up once real
  usage shows sessions commonly running enough concurrent agents to need
  it — noted here so it isn't lost, not built in v1.
