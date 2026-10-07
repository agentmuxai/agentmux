// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The fields of a raw stream line this reads. */
interface StreamLineShape {
    type?: string;
    subtype?: string;
    parent_tool_use_id?: string | null;
}

/**
 * Spots a new CLI session in the main agent's stream: a `system/init` that
 * doesn't belong to a compaction. Claude Code also writes an `init` right
 * after a `compact_boundary`, in the same turn, when it continues on the
 * compacted conversation; that one is not a new session. Fed every line in
 * stream order; returns true for the `init` of a new session.
 *
 * In stream order a new session's `init` comes after the last turn's `result`
 * and before the next turn's calls, so a turn still holding live tokens there
 * ended without a `result` (its process died).
 */
export function createSessionStartDetector(): (rawEvent: StreamLineShape) => boolean {
    let afterCompaction = false;
    return (rawEvent) => {
        if (rawEvent.type !== "system" || rawEvent.parent_tool_use_id) return false;
        if (rawEvent.subtype === "compact_boundary") {
            afterCompaction = true;
            return false;
        }
        if (rawEvent.subtype !== "init") return false;
        if (afterCompaction) {
            afterCompaction = false;
            return false;
        }
        return true;
    };
}
