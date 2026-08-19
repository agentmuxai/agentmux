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
 * Session-local only — no persistence across AgentMux restarts.
 * Stretch goal noted in SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §7.
 *
 * Spec: SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §5.1.
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

interface TokenUsageState {
    sessionStartAt: number;
    byService: Record<string, ServiceUsage>;
}

const [state, setState] = createStore<TokenUsageState>({
    sessionStartAt: Date.now(),
    byService: {},
});

/**
 * Record a completed turn's tokens under `provider`. No-op if the
 * tokens are missing or both counts are zero (nothing to aggregate).
 */
export function recordTurn(provider: string, tokens: ServiceUsage | null | undefined): void {
    if (!provider || !tokens) return;
    const input = tokens.input ?? 0;
    const output = tokens.output ?? 0;
    if (input === 0 && output === 0) return;
    const id = provider.toLowerCase();
    const current = state.byService[id] ?? { input: 0, output: 0 };
    // Breakdown fields accumulate only when provided — `?? 0` on the
    // current side (not the incoming side) so a provider that has never
    // reported a breakdown keeps reading as `undefined` (unknown) rather
    // than silently becoming `0` (known-zero) the moment ANY turn for it
    // omits the fields. Once any turn does supply the fields for this
    // service, the running total is exact from that point on.
    const hasBreakdown =
        tokens.freshInput != null || tokens.cacheCreation != null || tokens.cacheRead != null;
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
    // `input` IS the fresh count.
    const freshContribution =
        tokens.freshInput
        ?? ((tokens.cacheCreation != null || tokens.cacheRead != null) ? input : undefined);
    setState("byService", id, {
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
    });
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
    const services = state.byService;
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
    });
}

/** Accessor for reactive reads — components should read via this. */
export const tokenUsageState = state;
