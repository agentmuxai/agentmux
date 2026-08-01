// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared parsing for Claude Code's `system`/`compact_boundary` raw
 * stream-json frame — the authoritative compaction-completion event
 * (see docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md
 * §2/§4.1). This frame carries no `StreamEvent` shape in the provider
 * translator (mirrors `agentmux-srv`'s `translator/claude.rs`, which
 * only started handling `system` frames for this one subtype), so
 * both consumers intercept the raw frame directly rather than routing
 * through `translator.translate()`:
 *
 * - `useAgentStream.ts` — the live NDJSON subscription.
 * - `parseHistoryLines.ts` — the persisted-history replay pipeline.
 *
 * Extracted into one place (Codex P1, PR #2378 round 2: the replay
 * pipeline originally had no equivalent handling at all, so every
 * historical `compact_boundary` — and its exact token/duration record
 * — silently disappeared from a reopened pane's transcript) so the
 * two consumers can't drift on what counts as a valid frame.
 */

export interface CompactBoundaryData {
    trigger: "manual" | "auto";
    preTokens: number;
    postTokens: number;
    durationMs: number;
    /**
     * The frame's own top-level `timestamp` field, verbatim (raw string,
     * not parsed/reformatted) — `null` if absent or not a string. Used
     * ONLY to build a stable node id shared with `parseHistoryLines.ts`'s
     * own `context-compacted-${rawEvent.timestamp}` id (Codex P2, PR
     * #2378 round 7): the live path previously used `Date.now()`, so a
     * `compact_boundary` processed live AND later re-seen in a mount-time
     * history range (a real race — the two requests are independent)
     * produced two different ids for the same event, and the document
     * store's same-id dedup couldn't merge them — the compaction showed
     * up twice. NOT used for anything time-arithmetic (`state.at`,
     * watchdog timestamps, etc. all still use `Date.now()` at the call
     * site) — this is purely a dedup key.
     */
    frameTimestamp: string | null;
}

/**
 * Validate + extract a `compact_boundary` frame's `compactMetadata`,
 * or `null` if this isn't one, or its fields don't match the expected
 * shape. Malformed/missing fields degrade to `null` (skip rather than
 * guess) — same philosophy as `agentmux-srv`'s `claude.rs` translator.
 */
export function parseCompactBoundaryFrame(rawEvent: unknown): CompactBoundaryData | null {
    if (!rawEvent || typeof rawEvent !== "object") return null;
    const e = rawEvent as Record<string, unknown>;
    if (e.type !== "system" || e.subtype !== "compact_boundary") return null;

    const meta = e.compactMetadata as Record<string, unknown> | undefined;
    const trigger: "manual" | "auto" | null =
        meta?.trigger === "auto" ? "auto" :
        meta?.trigger === "manual" ? "manual" :
        null;
    const preTokens = typeof meta?.preTokens === "number" ? meta.preTokens : null;
    const postTokens = typeof meta?.postTokens === "number" ? meta.postTokens : null;
    const durationMs = typeof meta?.durationMs === "number" ? meta.durationMs : null;
    if (trigger == null || preTokens == null || postTokens == null || durationMs == null) {
        return null;
    }
    const frameTimestamp = typeof e.timestamp === "string" ? e.timestamp : null;
    return { trigger, preTokens, postTokens, durationMs, frameTimestamp };
}

/**
 * Stable `context_compacted` node id for a REAL `compact_boundary`, shared
 * by both consumers so they can never independently drift — the exact bug
 * class this module already exists to prevent (see the module doc
 * comment). Codex P2, PR #2378 round 12: previously each consumer built
 * its OWN fallback for the (defensive-only — the real CLI always sends a
 * timestamp, but both consumers parse defensively) timestamp-less case:
 * `useAgentStream.ts` used `Date.now()`, `parseHistoryLines.ts` used a
 * batch-relative node count. The same underlying boundary seen live AND
 * via a history-replay overlap could then get two different ids,
 * bypassing the document store's same-id dedup. Falls back to a
 * content-derived key instead — computed identically by both consumers
 * since both now call this one function with the same fields.
 */
export function contextCompactedNodeId(data: {
    trigger?: "manual" | "auto";
    preTokens: number;
    postTokens: number;
    durationMs?: number;
    frameTimestamp?: string | null;
}): string {
    const suffix =
        data.frameTimestamp ??
        `notime-${data.trigger ?? "?"}-${data.preTokens}-${data.postTokens}-${data.durationMs ?? "?"}`;
    return `context-compacted-${suffix}`;
}

/**
 * Epoch-ms completion time for a REAL `compact_boundary`'s
 * `context_compacted` node — parses `frameTimestamp` (the frame's own
 * completion time) when present/valid, falling back to `Date.now()` only
 * when it's absent or unparseable (codex P2, PR #2378 round 12: the live
 * path previously always recorded the frontend's receipt time here,
 * discarding the frame's own completion time even when available — and
 * since the live/replay nodes now share an id, a later history replay
 * can't correct a live node that already won the document store's dedup).
 * `parseHistoryLines.ts` deliberately does NOT reuse this fallback — a
 * timestamp-less line replayed from history should read as unknown (0),
 * not silently claim to have happened "now".
 */
export function contextCompactedLiveTimestamp(frameTimestamp: string | null | undefined): number {
    const parsed = typeof frameTimestamp === "string" ? Date.parse(frameTimestamp) : NaN;
    return Number.isNaN(parsed) ? Date.now() : parsed;
}
