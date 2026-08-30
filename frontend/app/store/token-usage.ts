// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Session-local token usage store.
 *
 * Aggregates TurnTokens across every completed turn in the current
 * AgentMux session, keyed by provider id ("claude", "codex", "gemini",
 * "kimi", "pi", "openclaw", "copilot", or any future agent id). The
 * status-bar indicator reads the total; the breakdown popover reads
 * per-service detail. Resets to zero on user action.
 *
 * Also aggregates the same turns keyed by agent (`byAgent`, added by
 * SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30.md) — real agent turns
 * (the only call site with pane identity, useTurnLifecycle.ts) are keyed
 * by blockId; the four ambient/internal call sites (background
 * suggestions, activity summaries, subagent naming — all pass no `agent`
 * argument) collapse into a single "__ambient__" bucket instead of
 * appearing as peer rows next to real agents.
 *
 * Session-local only — no persistence across AgentMux restarts.
 * Stretch goal noted in SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §7.
 *
 * Spec: SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §5.1,
 * SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30.md.
 */

import { createStore } from "solid-js/store";

export interface ServiceUsage {
    input: number;
    output: number;
    /**
     * Breakdown of `input` by cache status (fresh + cacheCreation + cacheRead
     * === input). Optional — providers without a structured cache signal
     * (codex/gemini/copilot) never set these, so consumers must treat them
     * as "unknown," not "zero." See TurnTokens'/SessionStats' doc comments
     * in view/agent/types.ts for the source of this split, and
     * docs/reports/REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md
     * §2.2/§5.2/§5.3 for why it's tracked (cache_read tokens cost ~0.1x a
     * fresh token — the breakdown is what actually explains cost, the raw
     * input number alone doesn't).
     */
    freshInput?: number;
    cacheCreation?: number;
    cacheRead?: number;
}

/**
 * One agent's (or the ambient bucket's) running totals — see the
 * module doc comment above. `byService` mirrors the top-level shape,
 * nested, to cover a pane that forks across providers mid-session
 * (rare, but real agent turns still carry a `provider`, so it's free to
 * keep).
 */
export interface AgentUsage {
    agentName: string;
    blockId: string | null;
    isAmbient: boolean;
    input: number;
    output: number;
    costUsd: number;
    numTurns: number;
    freshInput?: number;
    cacheCreation?: number;
    cacheRead?: number;
    byService: Record<string, ServiceUsage>;
}

/** Context describing which agent a real (non-ambient) turn belongs to. */
export interface AgentTurnContext {
    blockId: string;
    agentName: string;
    costUsd?: number;
}

const AMBIENT_KEY = "__ambient__";

interface TokenUsageState {
    sessionStartAt: number;
    byService: Record<string, ServiceUsage>;
    byAgent: Record<string, AgentUsage>;
}

const [state, setState] = createStore<TokenUsageState>({
    sessionStartAt: Date.now(),
    byService: {},
    byAgent: {},
});

function accumulateServiceUsage(current: ServiceUsage, tokens: ServiceUsage): ServiceUsage {
    const input = tokens.input ?? 0;
    const output = tokens.output ?? 0;
    const hasBreakdown =
        tokens.freshInput != null || tokens.cacheCreation != null || tokens.cacheRead != null;
    // Same normalization recordTurn already applies below (two incompatible
    // wire shapes reach this under the same field names) — see recordTurn's
    // own doc comment for the full rationale; kept in sync here since both
    // the service-keyed and agent-keyed aggregates need it identically.
    const freshContribution =
        tokens.freshInput
        ?? ((tokens.cacheCreation != null || tokens.cacheRead != null) ? input : undefined);
    return {
        input: current.input + input,
        output: current.output + output,
        freshInput: hasBreakdown
            ? (current.freshInput ?? 0) + (freshContribution ?? 0)
            : current.freshInput,
        cacheCreation: hasBreakdown
            ? (current.cacheCreation ?? 0) + (tokens.cacheCreation ?? 0)
            : current.cacheCreation,
        cacheRead: hasBreakdown
            ? (current.cacheRead ?? 0) + (tokens.cacheRead ?? 0)
            : current.cacheRead,
    };
}

/**
 * Record a completed turn's tokens under `provider`, and — for real
 * agent turns — under the agent identified by `agent`. Omitting `agent`
 * (the four ambient/internal call sites) files the turn under the
 * shared ambient bucket instead. No-op if the tokens are missing or both
 * counts are zero (nothing to aggregate).
 */
export function recordTurn(
    provider: string,
    tokens: ServiceUsage | null | undefined,
    agent?: AgentTurnContext,
): void {
    if (!provider || !tokens) return;
    const input = tokens.input ?? 0;
    const output = tokens.output ?? 0;
    if (input === 0 && output === 0) return;
    const id = provider.toLowerCase();
    // Breakdown fields accumulate only when provided by THIS turn — see
    // accumulateServiceUsage's use of `hasBreakdown` — so a provider that
    // has never reported a breakdown keeps reading as `undefined`
    // (unknown) rather than silently becoming `0` (known-zero) the moment
    // any turn for it omits the fields.
    //
    // reagentx/codex P2 on PR #2658: two incompatible wire shapes reach
    // this function under the same field names. claude-translator.ts /
    // useAgentStream.ts (via useTurnLifecycle.ts) pass `input` as the
    // COLLAPSED total (freshInput + cacheCreation + cacheRead) with a
    // separate `freshInput` for the fresh-only count. The backend
    // TokenCounts shape — gotypes.d.ts's `{input, output, cacheCreation,
    // cacheRead}`, no `freshInput` field at all, reaching recordTurn via
    // useNextPromptSuggestion/useAgentActivitySummary/ActivityDock/
    // swarm-view's ambient `result.tokens` — uses `input` to mean
    // fresh-only directly. Without this normalization, getCacheHitRate's
    // denominator silently drops that caller's fresh tokens (cacheCreation/
    // cacheRead are present, freshInput isn't), inflating the reported
    // hit rate — e.g. 100 fresh + 900 cache-read reads as 100%, not 90%.
    // Distinguishing rule: our own path always sets freshInput whenever it
    // sets cacheCreation/cacheRead (see useAgentStream.ts/reducer.ts — all
    // three are sourced together or not at all), so "cache fields present,
    // freshInput absent" unambiguously means the TokenCounts shape, where
    // `input` IS the fresh count. accumulateServiceUsage applies this same
    // normalization to both the byService and byAgent aggregates below.
    setState("byService", id, (current) => accumulateServiceUsage(current ?? { input: 0, output: 0 }, tokens));

    const agentKey = agent?.blockId ?? AMBIENT_KEY;
    setState("byAgent", agentKey, (current) => {
        const base: AgentUsage = current ?? {
            agentName: agent?.agentName ?? "AgentMux internal",
            blockId: agent?.blockId ?? null,
            isAmbient: agent == null,
            input: 0,
            output: 0,
            costUsd: 0,
            numTurns: 0,
            byService: {},
        };
        const serviceCurrent = base.byService[id] ?? { input: 0, output: 0 };
        const serviceNext = accumulateServiceUsage(serviceCurrent, tokens);
        const byServiceNext = { ...base.byService, [id]: serviceNext };
        // Summed across every service this agent has used (usually one —
        // a mid-session provider fork is the only way it's more than
        // one), not just the service this particular turn belongs to.
        const breakdown = sumBreakdown(byServiceNext);
        return {
            ...base,
            // A pane's agentName can't change turn-to-turn (one block, one
            // launch), but re-assert it anyway so a first turn recorded
            // before block.meta finished resolving doesn't strand the
            // fallback name for the rest of the session.
            agentName: agent?.agentName ?? base.agentName,
            input: base.input + input,
            output: base.output + output,
            costUsd: base.costUsd + (agent?.costUsd ?? 0),
            numTurns: base.numTurns + 1,
            freshInput: breakdown.freshInput,
            cacheCreation: breakdown.cacheCreation,
            cacheRead: breakdown.cacheRead,
            byService: byServiceNext,
        };
    });
}

/** Sum the cache breakdown fields across a services map — shared by the
 *  per-agent aggregation in recordTurn and getAgentCacheHitRate below.
 *  Fields stay `undefined` (unknown) rather than `0` when no service in
 *  the map has ever reported a breakdown, same "unknown vs. known-zero"
 *  distinction ServiceUsage's own doc comment establishes. */
function sumBreakdown(services: Record<string, ServiceUsage>): {
    freshInput?: number;
    cacheCreation?: number;
    cacheRead?: number;
} {
    let freshInput = 0;
    let cacheCreation = 0;
    let cacheRead = 0;
    let hasBreakdown = false;
    for (const id in services) {
        const u = services[id];
        if (u.freshInput == null && u.cacheCreation == null && u.cacheRead == null) continue;
        hasBreakdown = true;
        freshInput += u.freshInput ?? 0;
        cacheCreation += u.cacheCreation ?? 0;
        cacheRead += u.cacheRead ?? 0;
    }
    if (!hasBreakdown) return {};
    return { freshInput, cacheCreation, cacheRead };
}

/**
 * Sum of input + output across every service. Used by the
 * status-bar indicator. Renders as `↑Xk ↓Yk` via fmtTokens().
 */
export function getTotal(): ServiceUsage {
    const services = state.byService;
    let input = 0;
    let output = 0;
    let cacheRead = 0;
    let hasBreakdown = false;
    for (const id in services) {
        input += services[id].input;
        output += services[id].output;
        if (services[id].cacheRead != null) {
            hasBreakdown = true;
            cacheRead += services[id].cacheRead ?? 0;
        }
    }
    return { input, output, cacheRead: hasBreakdown ? cacheRead : undefined };
}

/**
 * Fraction of total prompt tokens (input + cacheCreation + cacheRead) served
 * from cache this session, across every service. `null` when no service has
 * reported a cache breakdown yet (e.g. session just started, or every active
 * provider is one that never reports cache fields) — render as "—", not "0%".
 * See docs/reports/REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md §5.3.
 */
export function getCacheHitRate(): number | null {
    return cacheHitRateOf(state.byService);
}

/** Same computation as getCacheHitRate, scoped to one agent row's own
 *  service breakdown instead of the whole session. */
export function getAgentCacheHitRate(row: AgentUsage): number | null {
    return cacheHitRateOf(row.byService);
}

function cacheHitRateOf(services: Record<string, ServiceUsage>): number | null {
    let promptTotal = 0;
    let cacheRead = 0;
    let hasBreakdown = false;
    for (const id in services) {
        const u = services[id];
        if (u.freshInput == null && u.cacheCreation == null && u.cacheRead == null) continue;
        hasBreakdown = true;
        promptTotal += (u.freshInput ?? 0) + (u.cacheCreation ?? 0) + (u.cacheRead ?? 0);
        cacheRead += u.cacheRead ?? 0;
    }
    if (!hasBreakdown || promptTotal === 0) return null;
    return cacheRead / promptTotal;
}

/**
 * Per-service breakdown sorted by total descending (biggest
 * consumer first). Stable secondary sort = service id alphabetical.
 */
export interface ServiceRow {
    id: string;
    input: number;
    output: number;
    freshInput?: number;
    cacheCreation?: number;
    cacheRead?: number;
}

export function getBreakdown(): ServiceRow[] {
    const rows: ServiceRow[] = Object.entries(state.byService).map(([id, u]) => ({
        id,
        input: u.input,
        output: u.output,
        freshInput: u.freshInput,
        cacheCreation: u.cacheCreation,
        cacheRead: u.cacheRead,
    }));
    rows.sort((a, b) => {
        const aTotal = a.input + a.output;
        const bTotal = b.input + b.output;
        if (bTotal !== aTotal) return bTotal - aTotal;
        return a.id.localeCompare(b.id);
    });
    return rows;
}

/**
 * Per-agent breakdown, sorted by cost descending (falls back to token
 * total when cost is 0/unknown for every row — some providers never
 * report `cost_usd`). Real agents always sort before the ambient
 * ("AgentMux internal") bucket, regardless of its size, since it isn't a
 * peer of a user's agents. See SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30.md.
 */
export function getAgentBreakdown(): AgentUsage[] {
    const rows = Object.values(state.byAgent);
    const anyCost = rows.some((r) => r.costUsd > 0);
    rows.sort((a, b) => {
        if (a.isAmbient !== b.isAmbient) return a.isAmbient ? 1 : -1;
        if (anyCost && a.costUsd !== b.costUsd) return b.costUsd - a.costUsd;
        const aTotal = a.input + a.output;
        const bTotal = b.input + b.output;
        if (bTotal !== aTotal) return bTotal - aTotal;
        return a.agentName.localeCompare(b.agentName);
    });
    return rows;
}

export function getSessionStartAt(): number {
    return state.sessionStartAt;
}

/**
 * Clear all running totals and reset `sessionStartAt` to now. Used
 * by the "Reset counter" button in the breakdown popover, gated by
 * a ConfirmModal since it's destructive.
 */
export function resetSession(): void {
    setState({
        sessionStartAt: Date.now(),
        byService: {},
        byAgent: {},
    });
}

/** Accessor for reactive reads — components should read via this. */
export const tokenUsageState = state;
