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
 *
 * How far it goes is decided by rows, not by one row: the fewest rows the strip can have is what it gets with every
 * compact form (one row when they fit, else two, ...); the fewest slots then collapse that reach that many rows
 * (of equal counts, the earliest in the order), so a pane keeps whatever full forms it has room for.
 */

/**
 * Collapse order, first to collapse first: the account (to its dot and initial), the context reading (to
 * "392k/1m", Compact to an icon), Shell (to an icon), the view chip (to an eye), then the CPU readout, last,
 * because its compact form is almost as wide as its full one.
 */
export const COMPACT_ORDER = ["auth", "ctx", "hostShell", "view", "stats"] as const;

/**
 * The keys that show their compact form. `full` holds every slot's width in its full form and `compact` the width
 * of those that have a compact form (a slot the strip hides measures 0 in `full` and takes no part). `rowCount`
 * says how many rows the strip's layout makes of the slots at the widths it is given. Picks the fewest slots to
 * collapse that bring the layout down to the rows it has with every compact form, and of the sets of that size
 * the one that comes first in `order`. Exhaustive: at most a handful of slots have a compact form.
 */
export function chooseCompactKeys(
    order: readonly string[],
    full: Record<string, number>,
    compact: Record<string, number>,
    rowCount: (widths: Record<string, number>) => number
): Set<string> {
    const candidates = order.filter((k) => (full[k] ?? 0) > 0 && compact[k] !== undefined && compact[k] < full[k]);
    const target = rowCount({ ...full, ...Object.fromEntries(candidates.map((k) => [k, compact[k]])) });
    let best: string[] = candidates;
    let bestCost = [candidates.length, Infinity];
    for (let mask = 0; mask < 1 << candidates.length; mask++) {
        const picked = candidates.filter((_, i) => mask & (1 << i));
        // Cost: how many collapse, then how late in the order they are (an earlier slot is the cheaper to give up).
        const cost = [picked.length, picked.reduce((sum, k) => sum + order.indexOf(k), 0)];
        if (cost[0] > bestCost[0] || (cost[0] === bestCost[0] && cost[1] >= bestCost[1])) continue;
        if (rowCount({ ...full, ...Object.fromEntries(picked.map((k) => [k, compact[k]])) }) > target + 1e-9) continue;
        best = picked;
        bestCost = cost;
    }
    return new Set(best);
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

/**
 * How many characters (the ellipsis counted) a tight account may show between its full label and its one letter,
 * widest first. The strip measures the account at each and gives it the widest that keeps the rows it has.
 */
export const ACCOUNT_BUDGETS = [15, 12, 9, 7] as const;

/**
 * An address cut to `budget` characters in the middle, the ellipsis counted: the start of the local part and the
 * whole domain when the domain fits ("taylo…@mail.dev"), else the start of the address ("taylor.m…"). A label that
 * already fits is left as it is.
 */
export function middleTruncate(label: string, budget: number): string {
    if (label.length <= budget) return label;
    const at = label.lastIndexOf("@");
    const domain = at > 0 ? label.slice(at) : "";
    const keep = budget - domain.length - 1;
    if (domain && keep >= 1) return `${label.slice(0, Math.min(keep, at))}…${domain}`;
    return `${label.slice(0, Math.max(1, budget - 1))}…`;
}

/**
 * The widest budget whose width still keeps the strip to the rows it has, or none (the account then shows its one
 * letter). `widthAt` is the account's measured width at a budget (none when it hasn't been measured), `fits` says
 * whether the strip keeps its rows with the account that wide.
 */
export function chooseMiddleBudget(
    budgets: readonly number[],
    widthAt: (budget: number) => number | undefined,
    fits: (width: number) => boolean
): number | undefined {
    return budgets.find((budget) => {
        const width = widthAt(budget);
        return width !== undefined && fits(width);
    });
}
