// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The shutdown log a closing agent pane shows while srv winds it down —
 * SPEC_AGENT_SELF_QUIT_2026_09_24.md §5.5, contract §12.1.
 *
 * `beginShutdownLog(blockId)` subscribes to that block's `agent:shutdown`
 * events BEFORE the close request goes out (srv publishes only after), and the
 * pane's overlay renders `shutdownLogFor(blockId)`. The pane itself leaves the
 * layout when srv's `delete` action arrives, not here.
 */

import { createStore, produce } from "solid-js/store";
import { MOS } from "@/app/store/global";
import { muxEventSubscribe } from "@/app/store/mps";

export const EVENT_AGENT_SHUTDOWN = "agent:shutdown";

/** One `agent:shutdown` line (§12.1). */
export type ShutdownLine = { seq: number; step: string; text: string; error?: string };

export type ShutdownLog = {
    lines: ShutdownLine[];
    /** Set by a `done` line: srv finished; the pane closes with its layout change. */
    done: boolean;
    /** Set by an `error` line: the pane stays open showing it. */
    error?: string;
};

/** Display cap (§12.1): the rest collapse into "+N more". */
export const SHUTDOWN_LOG_MAX_LINES = 8;

/** Pure: fold one event into a log — ordered by `seq`, duplicates ignored. */
export function applyShutdownEvent(log: ShutdownLog, data: unknown): ShutdownLog {
    const d = data as Partial<ShutdownLine> | undefined;
    if (!d || typeof d.seq !== "number" || typeof d.step !== "string") return log;
    if (log.lines.some((l) => l.seq === d.seq)) return log;
    const line: ShutdownLine = { seq: d.seq, step: d.step, text: String(d.text ?? ""), error: d.error };
    const lines = [...log.lines, line].sort((a, b) => a.seq - b.seq);
    return {
        lines,
        done: log.done || d.step === "done",
        error: d.step === "error" ? String(d.error ?? d.text ?? "couldn't close") : log.error,
    };
}

/** Pure: what to show — at most `max` lines, the newest kept, plus a count. */
export function visibleShutdownLines(lines: ShutdownLine[], max = SHUTDOWN_LOG_MAX_LINES): { shown: ShutdownLine[]; more: number } {
    // The terminal steps stay visible; the middle collapses.
    if (lines.length <= max) return { shown: lines, more: 0 };
    return { shown: lines.slice(lines.length - max), more: lines.length - max };
}

const [logs, setLogs] = createStore<Record<string, ShutdownLog | undefined>>({});
const unsubscribers = new Map<string, () => void>();

/** The log a pane should show, or undefined when it isn't closing. */
export function shutdownLogFor(blockId: string): ShutdownLog | undefined {
    return logs[blockId];
}

/**
 * Start showing the shutdown log for `blockId` and listen for its lines.
 * Idempotent: a second call keeps the existing log and subscription.
 */
export function beginShutdownLog(blockId: string): void {
    if (unsubscribers.has(blockId)) return;
    setLogs(blockId, { lines: [], done: false });
    const unsub = muxEventSubscribe({
        eventType: EVENT_AGENT_SHUTDOWN,
        scope: MOS.makeORef("block", blockId),
        handler: (event: { data?: unknown }) => {
            setLogs(
                produce((all) => {
                    const cur = all[blockId];
                    if (cur) all[blockId] = applyShutdownEvent(cur, event?.data);
                })
            );
        },
    });
    unsubscribers.set(blockId, unsub ?? (() => {}));
}

/** Stop showing it: the pane closed, or the user dismissed a failed close. */
export function endShutdownLog(blockId: string): void {
    unsubscribers.get(blockId)?.();
    unsubscribers.delete(blockId);
    setLogs(blockId, undefined);
}
