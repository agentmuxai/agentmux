// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared parsing for AgentMux's own `system`/`agentmux_session_outcome` raw
 * stream-json frame — emitted by the backend
 * (`agentmux-srv/src/backend/blockcontroller/persistent.rs`,
 * `session_outcome_line`) the moment a `--resume <sid>` attempt's fate
 * becomes definitively known (see
 * docs/specs/SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2). Mirrors
 * `compact-boundary.ts`'s shape exactly: the provider CLI would never emit
 * this frame itself, so both consumers — `useAgentStream.ts` (live) and
 * `parseHistoryLines.ts` (persisted-history replay) — intercept it directly
 * rather than routing through the generic provider `Translator`, which has
 * no shape for it.
 */

export interface SessionOutcomeData {
    outcome: "resumed" | "fresh";
    attemptedSid: string;
    actualSid: string | null;
    /**
     * The frame's own `timestamp` field, verbatim — used only to build a
     * stable node id shared by both consumers (same rationale as
     * `compact-boundary.ts`'s `frameTimestamp`: a live-seen event and the
     * same event re-seen via a later history-page overlap must land on the
     * same id, or the document store's dedup can't merge them).
     */
    frameTimestamp: string | null;
}

/**
 * Validate + extract an `agentmux_session_outcome` frame, or `null` if this
 * isn't one, or its fields don't match the expected shape. Malformed/missing
 * fields degrade to `null` (skip rather than guess) — same philosophy as
 * `compact-boundary.ts`'s `parseCompactBoundaryFrame`.
 */
export function parseSessionOutcomeFrame(rawEvent: unknown): SessionOutcomeData | null {
    if (!rawEvent || typeof rawEvent !== "object") return null;
    const e = rawEvent as Record<string, unknown>;
    if (e.type !== "system" || e.subtype !== "agentmux_session_outcome") return null;

    const outcome: "resumed" | "fresh" | null =
        e.outcome === "resumed" ? "resumed" : e.outcome === "fresh" ? "fresh" : null;
    const attemptedSid = typeof e.attempted_sid === "string" ? e.attempted_sid : null;
    if (outcome == null || attemptedSid == null) return null;

    const actualSid = typeof e.actual_sid === "string" ? e.actual_sid : null;
    const frameTimestamp = typeof e.timestamp === "string" ? e.timestamp : null;
    return { outcome, attemptedSid, actualSid, frameTimestamp };
}

/**
 * Stable `session_outcome` node id, shared by both consumers — same
 * content-derived fallback rationale as `compact-boundary.ts`'s
 * `contextCompactedNodeId`.
 */
export function sessionOutcomeNodeId(data: SessionOutcomeData): string {
    const suffix = data.frameTimestamp ?? `notime-${data.outcome}-${data.attemptedSid}`;
    return `session-outcome-${suffix}`;
}

/**
 * Epoch-ms time for a `session_outcome` node — parses `frameTimestamp` when
 * present/valid. Callers on the live path fall back to `Date.now()`
 * (mirrors `compact-boundary.ts`'s `contextCompactedLiveTimestamp`);
 * `parseHistoryLines.ts` deliberately does NOT use this fallback — a
 * timestamp-less replayed line should read as unknown (0), not claim to
 * have happened "now".
 */
export function sessionOutcomeLiveTimestamp(frameTimestamp: string | null | undefined): number {
    const parsed = typeof frameTimestamp === "string" ? Date.parse(frameTimestamp) : NaN;
    return Number.isNaN(parsed) ? Date.now() : parsed;
}

/**
 * Index of the LAST `session_outcome` node with `outcome: "fresh"` in an
 * ordered node list, or -1 if none. A `fresh` outcome is the working
 * scrollback's scope anchor: the model has none of the content before it,
 * so the working view must not show that content
 * (SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md
 * §3.1). `resumed` outcomes are NOT anchors — the model genuinely has the
 * prior turns. Structurally typed (not `DocumentNode[]`) so both the
 * document reducer and the pagination hook can call it without an import
 * cycle through the full node union.
 */
export function lastFreshBoundaryIndex(
    nodes: ReadonlyArray<{ type: string; outcome?: string }>,
): number {
    for (let i = nodes.length - 1; i >= 0; i--) {
        const n = nodes[i];
        if (n.type === "session_outcome" && n.outcome === "fresh") return i;
    }
    return -1;
}
