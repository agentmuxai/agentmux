// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane-open continuity notice injection for the LIVE working document — a
 * render-time synthetic (never persisted, never dispatched into the reducer
 * store), the same shape as `inject-history-link.ts`.
 *
 * Answers, at the top of the transcript, the question the pane could never
 * answer before the user typed: is the conversation below actually going to be
 * continued? See `hooks/useResumePreflight.ts`.
 */

import type { DocumentNode, ResumePreflightNode } from "./types";

/**
 * Build the notice node, or `null` when there's nothing worth saying.
 *
 * Silent by design in three cases, because a notice nobody needs is a notice
 * that trains people to ignore the ones they do:
 *   - `resume` / `unknown` verdicts — nothing is being lost, or we can't tell.
 *   - an empty document — there is no "conversation below" to warn about, so a
 *     brand-new agent's first open stays clean.
 *   - a document that already carries a real `session_outcome` divider — the
 *     spawn has happened and reported for itself; a *prediction* must never sit
 *     alongside the retrospective fact and risk contradicting it.
 */
export function buildResumePreflightNode(
    nodes: ReadonlyArray<DocumentNode>,
    preflight: SessionResumePreflightResult | null,
    pending: boolean,
): ResumePreflightNode | null {
    // Cheap rejections first. This runs on every document change, i.e. every
    // stream flush, and the `session_outcome` scan below is the only part that
    // walks the node list — so it must not run for the vast majority of panes,
    // which resolve to `resume` and have nothing to say from then on.
    const verdict =
        preflight?.verdict === "fresh" || preflight?.verdict === "recover"
            ? preflight.verdict
            : null;
    if (!pending && verdict == null) return null;
    if (nodes.length === 0) return null;
    if (nodes.some((n) => n.type === "session_outcome")) return null;

    if (pending) {
        return { type: "resume_preflight", id: "resume-preflight", pending: true, steps: [] };
    }
    if (!preflight || verdict == null) return null;

    return {
        type: "resume_preflight",
        id: "resume-preflight",
        verdict,
        recoverableSessionId: preflight.recoverable_session_id,
        pending: false,
        steps: preflight.steps ?? [],
    };
}

/**
 * Prepend the notice to the document. Front of the list on purpose: it's about
 * everything below it, and `injectHistoryLink` runs after this so the history
 * link keeps its own position relative to a `session_outcome` boundary.
 * Returns `nodes` unchanged when there's nothing to say.
 */
export function injectResumePreflight(
    nodes: ReadonlyArray<DocumentNode>,
    node: ResumePreflightNode | null,
): DocumentNode[] {
    if (!node) return nodes as DocumentNode[];
    return [node, ...nodes];
}
