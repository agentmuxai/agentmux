// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The token popover's AgentMux internal row (AmbientPurposes.tsx): how AgentMux's own
 * model calls (session titles, names, prompt suggestions, narration) have gone
 * since srv started, from `ambient.outcomes` (crates/srv/src/ambient/outcome.rs),
 * so "the swarm has been blank for an hour" is visible without copying a database.
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
    ["continuity_state", "Running summaries"],
    ["supervisor_verdict", "Supervisor verdicts"],
];

/** What a purpose does, as the object of "AgentMux can't …", for the dot's tip and the popover. */
const PURPOSE_WHAT: Readonly<Record<string, string>> = {
    activity_summary: "write the session title",
    activity_summary_pushed: "recover the session title",
    next_prompt_suggestion: "suggest the next prompt",
    subagent_name: "name a subagent",
    dispatch_name: "name a workflow",
    definition_summary: "write the agent preview",
    ambient_narration: "write the narration",
    continuity_state: "write the running summary",
    supervisor_verdict: "give the supervisor's verdict",
};

/** "write the session title" for a purpose; a purpose this build doesn't know is named by its tag. */
export function purposeWhat(purpose: string): string {
    return PURPOSE_WHAT[purpose] ?? `run ${purpose}`;
}

/** The latest failure of one purpose for one agent pane, from `ambient.failures` (srv). */
export interface AmbientFailure {
    purpose: string;
    /** The block or agent id the call was for. */
    entity: string;
    /** The agent's display name, when srv has it. */
    entity_name?: string | null;
    /** `cli_failed` or `timeout`. */
    outcome: string;
    /** One line of what the call said, bounded by srv. */
    reason?: string | null;
    /** srv's reading of the reason: out of credit, needs a sign-in, or anything else. */
    class?: "payment" | "auth" | "other" | null;
    /** Failures of this purpose for this entity since srv started. */
    count: number;
    /** Unix ms of the latest one. */
    last_at: number;
    /** Unix ms of the next try, while backing off. */
    retry_at?: number | null;
}

const countFmt = new Intl.NumberFormat("en-US");

/** "Deke", or the id srv gave when it has no name for it. */
export function failureEntity(f: AmbientFailure): string {
    return (f.entity_name ?? "").trim() || f.entity;
}

/** The state in two words when the reason says what the person has to do, else null. */
export function failureStateLabel(f: AmbientFailure): string | null {
    if (f.class === "payment") return "out of credit";
    if (f.class === "auth") return "needs sign-in";
    return null;
}

function timeOfDay(ms: number): string {
    return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** "Out of credit: AgentMux can't write the session title for Deke (failed 2,585 times since start)". */
export function describeFailure(f: AmbientFailure, now: number = Date.now()): string {
    const state = failureStateLabel(f);
    const lead = `AgentMux can't ${purposeWhat(f.purpose)} for ${failureEntity(f)}`;
    const times = `${f.outcome === "timeout" ? "timed out" : "failed"} ${countFmt.format(f.count)} time${f.count === 1 ? "" : "s"} since start`;
    const retry = f.retry_at != null && f.retry_at > now ? `, trying again at ${timeOfDay(f.retry_at)}` : "";
    const body = `${lead} (${times}${retry})`;
    return state ? `${state[0].toUpperCase()}${state.slice(1)}: ${body}` : body;
}

/** What the dot says: the first thing failing in plain words, and how many others. Failures name a
 *  pane; before srv has any, the purpose rows say the same without one. */
export function describeFailingTip(
    summary: AmbientOutcomeSummary | null,
    failures: ReadonlyArray<AmbientFailure>,
    now: number = Date.now()
): string {
    const failingPurposes = new Set((summary?.rows ?? []).filter((r) => r.unhealthy).map((r) => r.purpose));
    const named = failures.filter((f) => failingPurposes.has(f.purpose));
    if (named.length > 0) {
        const more = named.length - 1;
        return `${describeFailure(named[0], now)}${more > 0 ? ` +${more} more` : ""}`;
    }
    const rows = (summary?.rows ?? []).filter((r) => r.unhealthy);
    if (rows.length === 0) return "";
    const first = rows[0];
    const bad = first.counts.failed > 0
        ? `failed ${countFmt.format(first.counts.failed)} time${first.counts.failed === 1 ? "" : "s"} since start`
        : `refused ${countFmt.format(first.counts.refused)} time${first.counts.refused === 1 ? "" : "s"} since start`;
    const more = rows.length - 1;
    return `AgentMux can't ${purposeWhat(first.purpose)} (${bad})${more > 0 ? ` +${more} more` : ""}`;
}

export type AmbientBucket = "accepted" | "kept" | "refused" | "skipped" | "failed";

/** Which bucket an outcome label counts toward; `null` for a label this build
 *  doesn't know, which still counts toward the total and shows in the hover. */
function bucketOf(label: string): AmbientBucket | null {
    if (label === "accepted") return "accepted";
    // `skipped` is the model's SKIP: like `kept`, a healthy "nothing to write".
    if (label === "kept" || label === "skipped") return "kept";
    if (label.startsWith("rejected")) return "refused";
    // `gated`: a check in code showed there was nothing to ask, so no call.
    if (label === "empty_digest" || label === "superseded" || label === "gated") return "skipped";
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
    /** The same counts by bucket. A label this build doesn't know is in `total` only. */
    counts: Record<AmbientBucket, number>;
    /** Every outcome label and its count, one per line, for the hover. */
    detail: string;
    total: number;
    /** Refusals and failures outnumber the useful answers (accepted or kept). */
    unhealthy: boolean;
}

export interface AmbientOutcomeSummary {
    rows: AmbientPurposeRow[];
    /** Every request that ended, including those that never reached the model
     *  (`empty_digest`, `not_run`, an early `superseded`): requests, not calls. */
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
        const n: Record<AmbientBucket, number> = { accepted: 0, kept: 0, refused: 0, skipped: 0, failed: 0 };
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
            counts: n,
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
