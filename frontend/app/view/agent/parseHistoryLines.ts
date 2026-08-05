// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * parseHistoryLines — batch-parse a slice of persisted NDJSON lines into
 * DocumentNodes using the same pipeline as useAgentStream.
 *
 * This is a pure function (no signals, no side-effects) so it can be called
 * from onMount without touching SolidJS reactivity.
 */

import { createTranslator } from "./providers/translator-factory";
import { ClaudeCodeStreamParser } from "./stream-parser";
import { parseCompactBoundaryFrame, contextCompactedNodeId } from "./compact-boundary";
import { parseSessionOutcomeFrame, sessionOutcomeNodeId } from "./session-outcome";
import type { ContextCompactedNode, DocumentNode, SessionOutcomeNode, SessionStats } from "./types";

export interface ParsedHistory {
    nodes: DocumentNode[];
    /**
     * Stats payload of the last `session_end` event seen during replay that
     * actually carries token usage, or `null` if none was found. Claude's
     * persistent-mode controller emits a `session_end` after EVERY plain-text
     * turn as a boundary marker with `stats: {}` (see
     * ClaudeTranslator.handleAssistantMessage) — the real usage-bearing
     * `result` event only fires at process teardown, which for a
     * long-running persistent session may be far earlier in the replayed
     * window than the last turn boundary. Tracking the chronologically last
     * `session_end` unconditionally would let an empty boundary marker
     * clobber real historical stats. Callers use this to seed the composer
     * strip's context-fill bar at mount (see useAgentStream's `finalizeTurn`
     * / `TokensIn` for the live-stream equivalent).
     */
    lastSessionStats: SessionStats | null;
}

/**
 * Parse an array of raw NDJSON lines (as stored in the "output" blockfile)
 * and return the resulting DocumentNodes.
 *
 * @param lines        Raw text lines from blockfile:read_range
 * @param outputFormat Provider output format string (e.g. "claude-stream-json")
 * @param agentName    This pane's agent name (`block.meta.agentName`).
 *                     Threaded to `parser.setAgentId` so jekt direction
 *                     detection works during replay — an echoed outgoing jekt
 *                     (FROM == this agent) must rebuild as an outgoing bubble.
 *                     Optional; missing name falls back to "incoming".
 * @returns            Ordered DocumentNodes (deduped by node id) plus the
 *                      last `session_end` stats payload found, if any.
 */
export function parseHistoryLines(
    lines: string[],
    outputFormat: string,
    agentName?: string,
): ParsedHistory {
    const translator = createTranslator(outputFormat);
    // isReplay: true — thinking/tool_call events carry no wire timestamp of
    // their own, so stamping Date.now() here would show every replayed
    // clump/call as "just now" (reagent P2 on PR #2392). See
    // ClaudeCodeStreamParser's isReplay doc comment.
    const parser = new ClaudeCodeStreamParser({ isReplay: true });
    if (agentName) parser.setAgentId(agentName);
    const nodes: DocumentNode[] = [];
    let lastSessionStats: SessionStats | null = null;
    // Same-id events update IN PLACE rather than first-wins.
    // The previous "skip if seen" rule dropped legitimate state
    // transitions during replay — most importantly, a `tool_result`
    // arriving after its `tool_call` would be discarded, leaving the
    // tool stuck at `status: "running"` on the rendered page even
    // though it completed successfully. Codex P1 on PR #1104. Streaming
    // markdown/thinking deltas also share an id; same rule means the
    // accumulated tail (the last delta containing the full text) wins,
    // which is the same end state the live stream produces.
    const indexById = new Map<string, number>();

    for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed || !trimmed.startsWith("{")) continue;

        let rawEvent: any;
        try {
            rawEvent = JSON.parse(trimmed);
        } catch {
            // Corrupt line — skip silently (same as useAgentStream behaviour)
            continue;
        }

        // Handle stderr events (unlikely in persisted history, but be safe)
        if (rawEvent.type === "stderr") continue;

        // Real compaction-boundary completion data. Same raw-frame
        // interception as useAgentStream.ts's live path (shared parsing
        // via compact-boundary.ts) — this frame has no StreamEvent shape
        // in the provider translator, so without this the replay pipeline
        // silently dropped every historical compact_boundary along with
        // its exact token/duration record (Codex P1, PR #2378 round 2).
        if (rawEvent.type === "system" && rawEvent.subtype === "compact_boundary") {
            // Bypassing parser.parseLine() below means the parser's
            // currentTextNode/currentThinkingNode accumulator never sees
            // this line — without closing it explicitly, text AFTER the
            // boundary would keep accumulating onto the SAME node id as
            // text BEFORE it (same bug class as Codex P1 #1104's
            // tool_call/tool_result merge issue, just for the text
            // accumulator instead), silently merging content across the
            // compaction and reordering it before the compaction marker
            // in the replayed transcript. Flushed unconditionally — even
            // a boundary frame whose metadata fails to parse below is
            // still a real boundary in the underlying conversation.
            // flushPending()'s return value is discarded: those nodes are
            // already correctly represented in `nodes` from when the
            // per-line loop processed them.
            parser.flushPending();
            const data = parseCompactBoundaryFrame(rawEvent);
            if (data) {
                const parsedTs = typeof rawEvent.timestamp === "string" ? Date.parse(rawEvent.timestamp) : NaN;
                const node: ContextCompactedNode = {
                    type: "context_compacted",
                    // Codex P2, PR #2378 round 12: shares useAgentStream.ts's
                    // exact id-construction function (including its
                    // content-derived fallback for the timestamp-less
                    // case) instead of independently reimplementing it here
                    // with a different fallback (previously nodes.length,
                    // a batch-relative counter) — the same underlying
                    // boundary seen live AND via a history-replay overlap
                    // must always land on the identical id, or the
                    // document store's same-id dedup can't merge them.
                    id: contextCompactedNodeId(data),
                    tokensBefore: data.preTokens,
                    tokensAfter: data.postTokens,
                    timestamp: Number.isNaN(parsedTs) ? 0 : parsedTs,
                    source: "real",
                    trigger: data.trigger,
                    durationMs: data.durationMs,
                };
                const existing = indexById.get(node.id);
                if (existing != null) {
                    nodes[existing] = node;
                } else {
                    indexById.set(node.id, nodes.length);
                    nodes.push(node);
                }
            }
            continue;
        }

        // AgentMux's own resume-outcome marker — same raw-frame interception
        // as useAgentStream.ts's live path (shared parsing via
        // session-outcome.ts). See
        // docs/specs/SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md §2.2.
        if (rawEvent.type === "system" && rawEvent.subtype === "agentmux_session_outcome") {
            parser.flushPending();
            const data = parseSessionOutcomeFrame(rawEvent);
            if (data) {
                const parsedTs = typeof rawEvent.timestamp === "string" ? Date.parse(rawEvent.timestamp) : NaN;
                const node: SessionOutcomeNode = {
                    type: "session_outcome",
                    // Shares useAgentStream.ts's exact id-construction
                    // function — same rationale as context_compacted above.
                    id: sessionOutcomeNodeId(data),
                    outcome: data.outcome,
                    attemptedSid: data.attemptedSid,
                    actualSid: data.actualSid,
                    timestamp: Number.isNaN(parsedTs) ? 0 : parsedTs,
                };
                const existing = indexById.get(node.id);
                if (existing != null) {
                    nodes[existing] = node;
                } else {
                    indexById.set(node.id, nodes.length);
                    nodes.push(node);
                }
            }
            continue;
        }

        // Translate provider-specific envelope → StreamEvent[]
        const streamEvents = translator.translate(rawEvent);

        for (const event of streamEvents) {
            if (event.type === "session_end") {
                // Only overwrite when this session_end actually carries usage —
                // skip the empty-stats per-turn boundary marker so it can't
                // clobber a real result's stats seen earlier in the window.
                if (event.stats
                    && (typeof event.stats.input_tokens === "number"
                        || typeof event.stats.output_tokens === "number")) {
                    lastSessionStats = event.stats;
                }
                continue;
            }
            const node = parser.parseLine(JSON.stringify(event));
            if (!node) continue;
            const existing = indexById.get(node.id);
            if (existing != null) {
                // Replace at the original position so insertion order
                // tracks where the id first appeared (which is where
                // the live render would have placed it).
                nodes[existing] = node;
            } else {
                indexById.set(node.id, nodes.length);
                nodes.push(node);
            }
        }
    }

    return { nodes, lastSessionStats };
}
