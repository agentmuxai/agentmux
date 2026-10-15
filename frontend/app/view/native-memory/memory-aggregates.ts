// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Memory pane's aggregates over what it already holds: top contributors,
 * the file-size distribution and the fleet summary
 * (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.5). Medians, not means: one very
 * large file would make a mean misleading. Nothing here is stored.
 */

import { shareLabel, tokensLabel } from "./memory-tokens";

/** The middle value (the mean of the two middle ones for an even count); 0
 *  for none. */
export function median(values: number[]): number {
    if (values.length === 0) return 0;
    const s = [...values].sort((a, b) => a - b);
    const mid = s.length >> 1;
    return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

/** The nearest-rank `p`th percentile: the smallest value with at least `p`%
 *  of the values at or below it; 0 for none. */
export function percentile(values: number[], p: number): number {
    if (values.length === 0) return 0;
    const s = [...values].sort((a, b) => a - b);
    const rank = Math.ceil((Math.min(100, Math.max(0, p)) / 100) * s.length);
    return s[Math.max(0, rank - 1)];
}

/** The `n` largest of `tokens` and the total they are part of; `null` when
 *  there are no more than `n` (the "largest" would be all of them). */
export function topContributors(tokens: number[], n = 3): { count: number; tokens: number; total: number } | null {
    if (tokens.length <= n) return null;
    const total = tokens.reduce((a, b) => a + b, 0);
    if (!(total > 0)) return null;
    const top = [...tokens].sort((a, b) => b - a).slice(0, n);
    return { count: n, tokens: top.reduce((a, b) => a + b, 0), total };
}

/** "The 3 largest entries are 61% of Global Memory". */
export function topContributorsLabel(t: { count: number; tokens: number; total: number }, noun: string, of: string): string {
    return `The ${t.count} largest ${noun} are ${shareLabel(t.tokens, t.total)} of ${of}`;
}

export interface FileDistribution {
    files: number;
    median: number;
    p90: number;
    /** Files larger than the p90 size. */
    overP90: number;
}

/** The size distribution of a set of files' tokens; `null` for none. */
export function fileDistribution(tokens: number[]): FileDistribution | null {
    if (tokens.length === 0) return null;
    const p90 = percentile(tokens, 90);
    return { files: tokens.length, median: median(tokens), p90, overP90: tokens.filter((t) => t > p90).length };
}

/** "Per file: median ~420 tok, p90 ~2.1k tok · 4 files over p90". */
export function fileDistributionLabel(d: FileDistribution): string {
    const head = `Per file: median ${tokensLabel(d.median)}, p90 ${tokensLabel(d.p90)}`;
    if (d.overP90 === 0) return head;
    return `${head} · ${d.overP90 === 1 ? "1 file" : `${d.overP90} files`} over p90`;
}

export interface FleetAgent {
    id: string;
    name: string;
    tokens: number;
}

export interface FleetSummary {
    agents: number;
    /** Agents with any Personal Memory. */
    withMemory: number;
    total: number;
    /** The median over the agents with memory. */
    medianPerAgent: number;
    /** Agents with more than twice that median, largest first. */
    outliers: FleetAgent[];
}

/** Personal Memory across the fleet; `null` when no agent has any. */
export function fleetSummary(agents: FleetAgent[]): FleetSummary | null {
    const withMemory = agents.filter((a) => a.tokens > 0);
    if (withMemory.length === 0) return null;
    const med = median(withMemory.map((a) => a.tokens));
    return {
        agents: agents.length,
        withMemory: withMemory.length,
        total: withMemory.reduce((sum, a) => sum + a.tokens, 0),
        medianPerAgent: med,
        outliers: withMemory.filter((a) => a.tokens > 2 * med).sort((a, b) => b.tokens - a.tokens),
    };
}

/** "~42k tok across 7 of 12 agents · median ~3.1k tok per agent". */
export function fleetSummaryLabel(f: FleetSummary): string {
    return `${tokensLabel(f.total)} across ${f.withMemory} of ${f.agents} ${f.agents === 1 ? "agent" : "agents"} · median ${tokensLabel(f.medianPerAgent)} per agent`;
}
