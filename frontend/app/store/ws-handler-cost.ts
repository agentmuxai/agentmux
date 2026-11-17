// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where the page's websocket time goes, by kind of message. `ws-slow-handler.ts` names a message that took 200 ms or
 * more; a stretch of thousands of 12 ms messages never reaches it, and is what keeps a page over its frame budget. This
 * tallies every message's listener time under its kind (an event by name, an RPC request by command, an RPC response),
 * and once a window (a minute) with enough handling in it, writes one `[perf] ws-handler-cost` line naming the kinds
 * that took the most. A quiet page writes nothing. Never message content.
 */

/** The window the tally is written for. */
export const COST_WINDOW_MS = 60_000;
/** A window whose messages took less than this in all writes no line. */
export const COST_MIN_TOTAL_MS = 500;
/** The kinds named in a line, most expensive first. */
export const COST_TOP_KINDS = 5;
/** More kinds than this in one window and the rest are counted under "other", so the tally stays bounded. */
export const COST_MAX_KINDS = 64;

interface Tally {
    n: number;
    totalMs: number;
    maxMs: number;
}

interface WireMessage {
    data?: unknown;
}

/** A message's kind without allocating a description: `event:<name>`, `rpc:<command>`, `rpc-response` or `other`. */
export function wsMessageKind(msg: unknown): string {
    const rpc = ((msg ?? {}) as WireMessage).data as Record<string, unknown> | null | undefined;
    if (rpc == null || typeof rpc !== "object") return "other";
    if (rpc.command === "eventrecv") {
        const name = (rpc.data as { event?: unknown } | null | undefined)?.event;
        return typeof name === "string" ? `event:${name.slice(0, 40)}` : "event:?";
    }
    if (typeof rpc.command === "string") return `rpc:${rpc.command.slice(0, 40)}`;
    if (typeof rpc.resid === "string") return "rpc-response";
    return "other";
}

interface Clock {
    now: () => number;
    log: (line: string) => void;
}

const realClock: Clock = { now: () => performance.now(), log: (line) => console.warn(line) };

/** `note(start, msg)` after each message's listener has run; it adds the time since `start` to the message's kind. */
export function createHandlerCostTally(clock: Clock = realClock) {
    let windowStart = clock.now();
    let tallies = new Map<string, Tally>();
    let messages = 0;
    let totalMs = 0;

    function flushWindow(at: number): void {
        if (totalMs >= COST_MIN_TOTAL_MS) {
            const top = [...tallies.entries()].sort((a, b) => b[1].totalMs - a[1].totalMs).slice(0, COST_TOP_KINDS);
            const parts = top.map(
                ([kind, t]) =>
                    `${kind} n=${t.n} total=${Math.round(t.totalMs)}ms mean=${(t.totalMs / t.n).toFixed(1)}ms max=${Math.round(t.maxMs)}ms`
            );
            clock.log(
                `[perf] ws-handler-cost window=${Math.round((at - windowStart) / 1000)}s messages=${messages} total=${Math.round(totalMs)}ms top: ${parts.join("; ")}`
            );
        }
        windowStart = at;
        tallies = new Map();
        messages = 0;
        totalMs = 0;
    }

    return function note(start: number, msg: unknown): void {
        const end = clock.now();
        const took = end - start;
        let kind = wsMessageKind(msg);
        if (!tallies.has(kind) && tallies.size >= COST_MAX_KINDS) kind = "other";
        let t = tallies.get(kind);
        if (t == null) {
            t = { n: 0, totalMs: 0, maxMs: 0 };
            tallies.set(kind, t);
        }
        t.n++;
        t.totalMs += took;
        t.maxMs = Math.max(t.maxMs, took);
        messages++;
        totalMs += took;
        if (end - windowStart >= COST_WINDOW_MS) flushWindow(end);
    };
}
