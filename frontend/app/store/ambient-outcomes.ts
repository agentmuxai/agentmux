// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm's "AgentMux AI" section: how AgentMux's own model calls (session
 * titles, names, prompt suggestions, narration) have gone since srv started,
 * from `ambient.outcomes` (crates/srv/src/ambient/outcome.rs), so "the swarm
 * has been blank for an hour" is visible without copying a database.
 * docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md section 5.8.
 */

/** Purpose → outcome label → count, as the RPC returns it. */
export type AmbientOutcomes = Record<string, Record<string, number>>;

/** Display names, in display order, for the purposes srv records
 *  (crates/srv/src/ambient/purpose.rs). A purpose not listed here still shows,
 *  after these, under its own tag. */
const PURPOSE_NAMES: ReadonlyArray<readonly [purpose: string, name: string]> = [
    ["activity_summary", "Session titles"],
    ["activity_summary_pushed", "Session titles (recovery)"],
    ["next_prompt_suggestion", "Prompt suggestions"],
    ["subagent_name", "Subagent names"],
    ["dispatch_name", "Workflow names"],
    ["definition_summary", "Agent previews"],
    ["ambient_narration", "Narration"],
];

type Bucket = "accepted" | "kept" | "refused" | "skipped" | "failed";

/** Which bucket an outcome label counts toward; `null` for a label this build
 *  doesn't know, which still counts toward the total and shows in the hover. */
function bucketOf(label: string): Bucket | null {
    if (label === "accepted") return "accepted";
    if (label === "kept") return "kept";
    if (label.startsWith("rejected")) return "refused";
    if (label === "empty_digest" || label === "superseded") return "skipped";
    if (label === "cli_failed" || label === "timeout" || label === "not_run") return "failed";
    return null;
}

export interface AmbientPurposeRow {
    purpose: string;
    name: string;
    /** e.g. "12 accepted · 30 kept · 2 refused · 1 failed". "Accepted" is a
     *  candidate the validator took, not a change: the pane may still keep the
     *  old title as a rewording, and recovery may lose a race (#4243). */
    text: string;
    /** Every outcome label and its count, one per line, for the hover. */
    detail: string;
    total: number;
    /** Refusals and failures outnumber the useful answers (accepted or kept). */
    unhealthy: boolean;
}

export interface AmbientOutcomeSummary {
    rows: AmbientPurposeRow[];
    total: number;
    unhealthyCount: number;
}

/** `null` until some call has ended at least once. */
export function summarizeAmbientOutcomes(outcomes: AmbientOutcomes | null | undefined): AmbientOutcomeSummary | null {
    if (!outcomes) return null;
    const known = new Set(PURPOSE_NAMES.map(([purpose]) => purpose));
    const extra = Object.keys(outcomes)
        .filter((purpose) => !known.has(purpose))
        .sort()
        .map((purpose) => [purpose, purpose] as const);
    const rows: AmbientPurposeRow[] = [];
    for (const [purpose, name] of [...PURPOSE_NAMES, ...extra]) {
        const by = outcomes[purpose];
        if (!by) continue;
        const n: Record<Bucket, number> = { accepted: 0, kept: 0, refused: 0, skipped: 0, failed: 0 };
        let total = 0;
        const lines: string[] = [];
        for (const [label, count] of Object.entries(by).sort(([a], [b]) => a.localeCompare(b))) {
            total += count;
            lines.push(`${label}: ${count}`);
            const bucket = bucketOf(label);
            if (bucket) n[bucket] += count;
        }
        if (total === 0) continue;
        const parts = [`${n.accepted} accepted`, `${n.kept} kept`];
        if (n.refused > 0) parts.push(`${n.refused} refused`);
        if (n.skipped > 0) parts.push(`${n.skipped} skipped`);
        if (n.failed > 0) parts.push(`${n.failed} failed`);
        const bad = n.refused + n.failed;
        rows.push({
            purpose,
            name,
            text: parts.join(" · "),
            detail: lines.join("\n"),
            total,
            unhealthy: bad > 0 && bad > n.accepted + n.kept,
        });
    }
    if (rows.length === 0) return null;
    return {
        rows,
        total: rows.reduce((sum, r) => sum + r.total, 0),
        unhealthyCount: rows.filter((r) => r.unhealthy).length,
    };
}
