// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * digest-accessory — pure derivation of the **session-digest accessory** row
 * for an agent pane: the AI one-line session summary, projected into the
 * shared Pane-Accessories row model.
 *
 * The digest is a single-row, meta-derived, transient accessory (it lives in
 * the `top-fixed` region). Like the fork set, it is a **pure function of a
 * source of truth** — the block's `session:digest_*` / `session:line_count`
 * meta plus the hook's transient `loading`/`dismissed`/`failed` signals — with
 * no parallel store, per the "derive from a source of truth" rule
 * (SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15 §5.3). The digest BAR-cell (a
 * later phase) renders this descriptor via `<PaneRow>`.
 *
 * Spec: docs/specs/SPEC_SESSION_DIGEST_AS_PANE_ACCESSORY_2026_06_15.md §3.
 */

/** New lines since the last digest at/after which the summary reads as stale. */
export const STALE_LINE_THRESHOLD = 20;

/**
 * The subset of block meta this module reads. Mirrors the keys written by the
 * `session:digest` RPC + `blockcontroller/session_stats.rs`.
 */
export interface DigestMeta {
    /** `session:digest_summary` — cached summary text. */
    summary?: string;
    /** `session:digest_generated_at` — Unix ms the cached digest was made. */
    generatedAt?: number;
    /** `session:digest_last_line_count` — line count when the digest was made. */
    digestLastLineCount?: number;
    /** `session:line_count` — total output lines this session (O(1)). */
    lineCount?: number;
}

/**
 * Transient, view-owned state from `useSessionDigest`. `summary`/`generatedAt`
 * here override the cached meta when present (a fresh fetch landed); `failed`
 * marks the last fetch as errored.
 */
export interface DigestState {
    loading: boolean;
    dismissed: boolean;
    failed?: boolean;
    summary?: string | null;
    generatedAt?: number | null;
}

/** Lifecycle status → drives the row's status accent (shared palette). */
export type DigestStatus = "generating" | "fresh" | "stale" | "failed";

/** The derived digest accessory row. `null` = render no row. */
export interface DigestAccessory {
    /** Stable row id, `digest:<blockId>`. */
    id: string;
    /** Row title: "Summarizing…" while generating, else the summary text. */
    title: string;
    status: DigestStatus;
    /** Unix ms the shown summary was generated (for relative-age meta). */
    generatedAt?: number;
    /** New lines since the digest was made (staleness signal; ≥0). */
    linesSinceDigest: number;
    /** True when `status === "stale"` — the summary has drifted from the convo. */
    stale: boolean;
    /** Offer the regenerate (↻) action. */
    canRegenerate: boolean;
    /** Offer the dismiss (×) action. */
    canDismiss: boolean;
}

function resolvedSummary(meta: DigestMeta, state: DigestState): string | null {
    // A live fetch result (state.summary) wins over the cached meta; an
    // explicit null from the hook (fetch returned empty/failed) clears it.
    if (state.summary !== undefined) return state.summary;
    return meta.summary && meta.summary.length > 0 ? meta.summary : null;
}

/**
 * Derive the digest accessory row for a pane, or `null` to render nothing.
 *
 * Returns `null` when:
 *  - the user dismissed the digest this session, or
 *  - there's no summary and none is being generated (the common fresh-pane
 *    case — zero cost, no row).
 *
 * Status mapping (SPEC_SESSION_DIGEST_AS_PANE_ACCESSORY §3.3):
 *  - `generating` while a fetch is in flight,
 *  - `failed` when the last fetch errored (and a prior summary is shown),
 *  - `stale` when ≥ STALE_LINE_THRESHOLD lines were added since the digest,
 *  - `fresh` otherwise.
 *
 * @param blockId the agent block id (for the row id).
 * @param meta    the block's digest/line-count meta.
 * @param state   the hook's transient loading/dismissed/failed + live summary.
 */
export function computeDigestAccessory(
    blockId: string,
    meta: DigestMeta,
    state: DigestState,
): DigestAccessory | null {
    if (state.dismissed) return null;

    const summary = resolvedSummary(meta, state);
    const loading = state.loading;

    // Empty state: nothing summarized and nothing in flight → no row.
    if (!summary && !loading) return null;

    const lineCount = meta.lineCount ?? 0;
    const lastDigestLineCount = meta.digestLastLineCount ?? 0;
    const linesSinceDigest = Math.max(0, lineCount - lastDigestLineCount);

    let status: DigestStatus;
    if (loading) {
        status = "generating";
    } else if (state.failed && summary) {
        // A refresh errored but we still have the prior summary to show.
        status = "failed";
    } else if (linesSinceDigest >= STALE_LINE_THRESHOLD) {
        status = "stale";
    } else {
        status = "fresh";
    }

    const generatedAt = state.generatedAt ?? meta.generatedAt ?? undefined;

    return {
        id: `digest:${blockId}`,
        title: loading ? "Summarizing…" : (summary as string),
        status,
        generatedAt: generatedAt && generatedAt > 0 ? generatedAt : undefined,
        linesSinceDigest,
        stale: status === "stale",
        canRegenerate: true,
        canDismiss: true,
    };
}
