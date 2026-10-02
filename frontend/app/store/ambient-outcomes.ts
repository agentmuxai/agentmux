// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Instance panel's one-line account of how session titles have gone since
 * srv started, from `ambient.outcomes` (crates/srv/src/ambient/outcome.rs), so
 * "the swarm has been blank for an hour" is visible without copying a database.
 * docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md section 5.8.
 */

/** Purpose → outcome label → count, as the RPC returns it. */
export type AmbientOutcomes = Record<string, Record<string, number>>;

/** The purposes that produce a session title: the pane's request and the
 *  backend's empty-title recovery. */
export const TITLE_PURPOSES = ["activity_summary", "activity_summary_pushed"];

export interface TitleOutcomeSummary {
    /** e.g. "12 new · 30 kept · 2 refused · 1 failed". */
    text: string;
    /** Every purpose and label, one per line, for the hover. */
    detail: string;
    /** True when refusals and failures outnumber new titles: worth a look. */
    unhealthy: boolean;
}

/** `null` until a title call has ended at least once. */
export function summarizeTitleOutcomes(outcomes: AmbientOutcomes | null | undefined): TitleOutcomeSummary | null {
    if (!outcomes) return null;
    let accepted = 0;
    let kept = 0;
    let refused = 0;
    let failed = 0;
    const lines: string[] = [];
    for (const purpose of TITLE_PURPOSES) {
        const by = outcomes[purpose];
        if (!by) continue;
        for (const [label, n] of Object.entries(by).sort(([a], [b]) => a.localeCompare(b))) {
            lines.push(`${purpose} ${label}: ${n}`);
            if (label === "accepted") accepted += n;
            else if (label === "kept") kept += n;
            else if (label.startsWith("rejected")) refused += n;
            else if (label === "cli_failed" || label === "timeout") failed += n;
        }
    }
    if (lines.length === 0) return null;
    const parts = [`${accepted} new`, `${kept} kept`];
    if (refused > 0) parts.push(`${refused} refused`);
    if (failed > 0) parts.push(`${failed} failed`);
    return { text: parts.join(" · "), detail: lines.join("\n"), unhealthy: refused + failed > accepted && refused + failed > 0 };
}
