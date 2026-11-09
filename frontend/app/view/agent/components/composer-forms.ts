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
 * compact form (one row when they fit, else two, ...); the slots then collapse only until the layout reaches that
 * many rows, so a pane that needs two rows keeps whatever full forms fit in them.
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
 * says how many rows the strip's layout makes of the slots at the widths it is given. Collapses in `COMPACT_ORDER`
 * until the layout is down to the rows it has with every compact form.
 */
export function chooseCompactKeys(
    order: readonly string[],
    full: Record<string, number>,
    compact: Record<string, number>,
    rowCount: (widths: Record<string, number>) => number
): Set<string> {
    const candidates = order.filter((k) => (full[k] ?? 0) > 0 && compact[k] !== undefined && compact[k] < full[k]);
    const widths = { ...full };
    const target = rowCount({ ...full, ...Object.fromEntries(candidates.map((k) => [k, compact[k]])) });
    const chosen = new Set<string>();
    for (const key of candidates) {
        if (rowCount(widths) <= target) break;
        chosen.add(key);
        widths[key] = compact[key];
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
