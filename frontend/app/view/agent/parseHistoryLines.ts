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
import type { DocumentNode } from "./types";

/**
 * Parse an array of raw NDJSON lines (as stored in the "output" blockfile)
 * and return the resulting DocumentNodes.
 *
 * @param lines        Raw text lines from blockfile:read_range
 * @param outputFormat Provider output format string (e.g. "claude-stream-json")
 * @returns            Ordered array of DocumentNodes, deduped by node id
 */
export function parseHistoryLines(
    lines: string[],
    outputFormat: string,
): DocumentNode[] {
    const translator = createTranslator(outputFormat);
    const parser = new ClaudeCodeStreamParser();
    const nodes: DocumentNode[] = [];
    const seen = new Set<string>();

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
            const node = parser.parseLine(JSON.stringify(event));
            if (!node) continue;
            if (seen.has(node.id)) continue;
            seen.add(node.id);
            nodes.push(node);
        }
    }

    return nodes;
}
