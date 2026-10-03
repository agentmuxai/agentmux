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
     * not parsed/reformatted) — `null` if absent or not a string. A time
     * value only (the node's time, the reducer's stale-boundary checks);
     * the CLI's stdout frame has none, so every reader falls back to its
     * receipt time. Never an id: see `uuid`.
     */
    frameTimestamp: string | null;
    /**
     * The frame's own `uuid` — the same in the stdout and transcript forms,
     * and unique per compaction — `null` if absent or not a string. The key
     * of every node and claim derived from this boundary, so the live and
     * replay paths agree on one id.
     */
    uuid: string | null;
}

/**
 * Validate + extract a `compact_boundary` frame's metadata, or `null` if
 * this isn't one, or its fields don't match the expected shape.
 * Malformed/missing fields degrade to `null` (skip rather than guess) —
 * same philosophy as `agentmux-srv`'s `claude.rs` translator. Reads both
 * forms the CLI writes: snake_case on stdout (`compact_metadata`,
 * `pre_tokens`, …), camelCase in its transcript.
 */
export function parseCompactBoundaryFrame(rawEvent: unknown): CompactBoundaryData | null {
    if (!rawEvent || typeof rawEvent !== "object") return null;
    const e = rawEvent as Record<string, unknown>;
    if (e.type !== "system" || e.subtype !== "compact_boundary") return null;

    const meta = (e.compact_metadata ?? e.compactMetadata) as Record<string, unknown> | undefined;
    const trigger: "manual" | "auto" | null =
        meta?.trigger === "auto" ? "auto" :
        meta?.trigger === "manual" ? "manual" :
        null;
    const num = (snake: string, camel: string): number | null => {
        const v = meta?.[snake] ?? meta?.[camel];
        return typeof v === "number" ? v : null;
    };
    const preTokens = num("pre_tokens", "preTokens");
    const postTokens = num("post_tokens", "postTokens");
    const durationMs = num("duration_ms", "durationMs");
    if (trigger == null || preTokens == null || postTokens == null || durationMs == null) {
        return null;
    }
    const frameTimestamp = typeof e.timestamp === "string" ? e.timestamp : null;
    const uuid = typeof e.uuid === "string" && e.uuid ? e.uuid : null;
    return { trigger, preTokens, postTokens, durationMs, frameTimestamp, uuid };
}

/**
 * Stable `context_compacted` node id for a REAL `compact_boundary`, shared
 * by both consumers so they can never independently drift — the exact bug
 * class this module already exists to prevent (see the module doc
 * comment). Keyed on the boundary's `uuid`; a frame without one falls back
 * to a content-derived key, computed identically by both consumers. Never
 * a receipt time or a batch-relative count: the same boundary seen live
 * and on replay would get two ids.
 */
export function contextCompactedNodeId(data: {
    trigger?: "manual" | "auto";
    preTokens: number;
    postTokens: number;
    durationMs?: number;
    uuid?: string | null;
}): string {
    const suffix =
        data.uuid ??
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
