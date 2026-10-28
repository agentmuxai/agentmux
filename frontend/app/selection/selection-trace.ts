// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The last transitions of the selection machine, for the next "I clicked and nothing happened".
 * A ring buffer, readable from the console as `__selectionTrace()`; a failed or superseded
 * intent is also a console line. The Swarm's own counters (rebuilds, presses without a click) are in
 * `util/select-trace.ts`.
 */

import type { FailReason, SelectionStateKind } from "./selection-machine";

export const TRACE_MAX = 50;

export interface SelectionTraceEntry {
    at: number;
    line: string;
}

export interface SelectionTrace {
    /** An intent started, or ended with `ended` (a failure reason, "superseded" or "settled"). */
    transition(t: {
        intentId: number;
        source: string;
        blockId: string;
        from: SelectionStateKind;
        to: SelectionStateKind;
        sinceStartMs: number;
        reason?: FailReason | "superseded";
        detail?: string;
    }): void;
    /** An intent ended without a state change of its own (the user moved elsewhere). */
    note(line: string): void;
    entries(): readonly SelectionTraceEntry[];
}

const short = (blockId: string) => blockId.slice(0, 4);

export function createSelectionTrace(now: () => number = Date.now, log: (line: string) => void = console.warn): SelectionTrace {
    const entries: SelectionTraceEntry[] = [];
    const push = (line: string, loud: boolean) => {
        entries.push({ at: now(), line });
        if (entries.length > TRACE_MAX) entries.shift();
        if (loud) log(line);
    };
    return {
        transition(t) {
            const why = t.reason ? ` ${t.reason}${t.detail ? ` (${t.detail})` : ""}` : "";
            push(
                `[select] id=${t.intentId} ${t.source} blk=${short(t.blockId)} ${t.from}->${t.to} +${t.sinceStartMs}ms${why}`,
                t.to === "failed"
            );
        },
        note: (line) => push(line, false),
        entries: () => entries,
    };
}
