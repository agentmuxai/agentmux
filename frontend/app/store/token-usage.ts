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
    setState("byService", id, {
        input: current.input + input,
        output: current.output + output,
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
    for (const id in services) {
        input += services[id].input;
        output += services[id].output;
    }
    return { input, output };
}

/**
 * Per-service breakdown sorted by total descending (biggest
 * consumer first). Stable secondary sort = service id alphabetical.
 */
export interface ServiceRow {
    id: string;
    input: number;
    output: number;
}

export function getBreakdown(): ServiceRow[] {
    const rows: ServiceRow[] = Object.entries(state.byService).map(([id, u]) => ({
        id,
        input: u.input,
        output: u.output,
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
