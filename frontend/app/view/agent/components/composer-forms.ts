// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The composer strip's forms: which slots show their compact form so the strip keeps to as few rows as it can.
 * Pure functions, no DOM; AgentComposerStrip measures both forms of each slot (hidden probes) and renders the form
 * this picks. The same idea as the top band's priority layout (`top-band/layout.ts`): everything full when it fits;
 * otherwise the slots collapse one at a time in a fixed order until the row fits.
 *
 * The order is what collapses first. Never hidden, in any form: the runtime trigger (mode, model, effort), the
 * context reading, the CPU and memory readout, and Shell (the strip's one action).
 */

/**
 * Collapse order, first to collapse first: the account (to its dot and initial), the context reading (to
 * "392k/1m", Compact to an icon), then Shell (to an icon). The CPU readout joins in part B, last, because its
 * compact form is almost as wide as its full one.
 */
export const COMPACT_ORDER = ["auth", "ctx", "hostShell", "stats"] as const;

/**
 * The keys that show their compact form. `full` holds every slot's width in its full form and `compact` the width
 * of those that have a compact form. A slot that measures 0 is shed by the strip's own rules and takes no part.
 * Collapses in `COMPACT_ORDER` until the slots, with a `gap` between each, fit one row of `width`; when even all
 * compact forms don't fit, they all stay compact and the row layout wraps what is left.
 */
export function chooseCompactKeys(
    order: readonly string[],
    full: Record<string, number>,
    compact: Record<string, number>,
    width: number,
    gap: number
): Set<string> {
    const shown = Object.keys(full).filter((k) => (full[k] ?? 0) > 0);
    let total = shown.reduce((sum, k) => sum + full[k], 0) + Math.max(0, shown.length - 1) * gap;
    const chosen = new Set<string>();
    for (const key of order) {
        if (total <= width) break;
        const smaller = compact[key];
        if (!shown.includes(key) || smaller === undefined || smaller >= full[key]) continue;
        chosen.add(key);
        total -= full[key] - smaller;
    }
    return chosen;
}

/** The context reading short: "392k / 1.0m" is "392k/1m". Anything else is left as it is. */
export function shortContextText(text: string): string {
    return text.replace(/\s*\/\s*/, "/").replace(/(\d)\.0(?=[a-z]*$)/i, "$1");
}

/** The one letter an account shows when it is tight: the first letter or digit of the label, upper case. */
export function accountInitial(label: string): string {
    const found = label.match(/[\p{L}\p{N}]/u);
    return found ? found[0].toUpperCase() : "?";
}
