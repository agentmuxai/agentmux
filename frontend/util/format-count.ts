// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared display formatting for counts (token counts, context-window fill,
 * etc.) — consolidates 7 independently-duplicated "format a number as Xk"
 * implementations found across the composer strip, footer, status bar,
 * swarm pane, and transcript renderer, none of which rolled over past "k"
 * for large totals. See
 * docs/specs/SPEC_TOKEN_STATS_NUMBER_FORMATTING_2026_08_02.md.
 */

const TIERS: ReadonlyArray<{ threshold: number; suffix: string }> = [
    { threshold: 1_000_000_000, suffix: "b" },
    { threshold: 1_000_000, suffix: "m" },
    { threshold: 1_000, suffix: "k" },
];

/**
 * Compact k/m/b abbreviation for an integer count. Precision rule (per
 * magnitude tier, matching the pre-existing TokenUsageIndicator/
 * TokenBreakdownPopover convention): one decimal place below 10x the tier,
 * integer at 10x and above. This caps the abbreviated mantissa at ~2
 * significant digits before the decimal (e.g. "9.9k", "10k", never
 * "9999.9k") — which is what makes the k→m→b rollover sufficient on its
 * own without ever needing a thousands-comma inside the abbreviated form
 * itself.
 */
export function formatCompactNumber(n: number): string {
    const abs = Math.abs(n);
    const sign = n < 0 ? "-" : "";
    for (const { threshold, suffix } of TIERS) {
        if (abs >= threshold) {
            const scaled = abs / threshold;
            const text = scaled < 10 ? scaled.toFixed(1) : String(Math.round(scaled));
            return `${sign}${text}${suffix}`;
        }
    }
    return `${n}`;
}

/**
 * Exact, comma-grouped form for tooltips / full-precision text — a thin,
 * named wrapper so every call site is visibly making the same formatting
 * decision rather than a bare `.toLocaleString()` sprinkled ad hoc.
 */
export function formatExactNumber(n: number): string {
    return n.toLocaleString();
}
