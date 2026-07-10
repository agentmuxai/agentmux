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
import type { DocumentNode, SessionStats } from "./types";

export interface ParsedHistory {
    nodes: DocumentNode[];
    /**
     * Stats payload of the LAST `session_end` event seen during replay, or
     * `null` if none was found. The live stream discards this same event's
     * stats after using them to hydrate the composer strip's context-fill
     * bar (see useAgentStream's `finalizeTurn` / `TokensIn`); history replay
     * used to discard it too, leaving the bar blank until the next live
     * turn. Callers use this to seed the bar immediately at mount instead.
     */
    lastSessionStats: SessionStats | null;
}

/**
 * Parse an array of raw NDJSON lines (as stored in the "output" blockfile)
 * and return the resulting DocumentNodes.
 *
 * @param lines        Raw text lines from blockfile:read_range
 * @param outputFormat Provider output format string (e.g. "claude-stream-json")
 * @returns            Ordered DocumentNodes (deduped by node id) plus the
 *                      last `session_end` stats payload found, if any.
 */
export function parseHistoryLines(
    lines: string[],
    outputFormat: string,
): ParsedHistory {
    const translator = createTranslator(outputFormat);
    const parser = new ClaudeCodeStreamParser();
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

        // Translate provider-specific envelope → StreamEvent[]
        const streamEvents = translator.translate(rawEvent);

        for (const event of streamEvents) {
            if (event.type === "session_end") {
                lastSessionStats = event.stats ?? null;
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
