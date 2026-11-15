// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Names the websocket message whose handling took long, so a stretch like the 28 s `DOMWebSocket.onmessage` frame of
 * 2026-11-15 says which message it was. The long-frame line (`perf/long-frames.ts`) has the listener and its time and
 * nothing about the message; this line is the other half: one `[perf] slow-ws-handler` per message that took
 * SLOW_WS_HANDLER_MS or more, with its kind, size and block.
 *
 * Two times: `sync` is the listener itself (parse, router, the handlers it called, and the store writes whose effects Solid
 * ran at once); `total` also covers the promise continuations queued while it ran (an awaited RPC response resumes
 * there), which Chromium counts in the same frame. A message whose `total` is far over its `sync` is slow in what
 * resumed after it, not in what it carried.
 */

/** A message handled in this long, or longer, is logged. */
export const SLOW_WS_HANDLER_MS = 200;
/** At most this many lines in a window, so a stretch of slow messages does not make the log pipe part of the problem. */
export const SLOW_WS_LOG_MAX = 20;
export const SLOW_WS_LOG_WINDOW_MS = 10_000;

interface WireMessage {
    eventtype?: unknown;
    data?: unknown;
}

const scopeBlock = (scopes: unknown): string | null => {
    if (!Array.isArray(scopes)) return null;
    const found = scopes.find((s) => typeof s === "string" && s.startsWith("block:"));
    return typeof found === "string" ? found.slice(0, 40) : null;
};

/**
 * What a message is, in a few words and without its content: `kind` (an event by name, an RPC request by command, an RPC
 * response by its id), the block an event is scoped to, the number of items an event carries when it is a list, and the
 * message's size in characters.
 */
export function describeWsMessage(msg: unknown, size: number): string {
    const parts: string[] = [];
    const wire = (msg ?? {}) as WireMessage;
    const rpc = (wire.data ?? {}) as Record<string, unknown>;
    if (typeof wire.eventtype === "string" && wire.eventtype !== "rpc") parts.push(`type=${wire.eventtype.slice(0, 40)}`);
    if (rpc.command === "eventrecv") {
        const ev = (rpc.data ?? {}) as Record<string, unknown>;
        parts.push(`kind=event:${typeof ev.event === "string" ? ev.event.slice(0, 60) : "?"}`);
        const block = scopeBlock(ev.scopes);
        if (block) parts.push(`block=${block}`);
        if (Array.isArray(ev.data)) parts.push(`items=${ev.data.length}`);
        else if (Array.isArray((ev.data as { updates?: unknown } | null)?.updates)) {
            parts.push(`items=${((ev.data as { updates: unknown[] }).updates).length}`);
        }
    } else if (typeof rpc.command === "string") {
        parts.push(`kind=rpc:${rpc.command.slice(0, 60)}`);
        if (typeof rpc.reqid === "string") parts.push(`reqid=${rpc.reqid.slice(0, 12)}`);
    } else if (typeof rpc.resid === "string") {
        parts.push(`kind=rpc-response resid=${rpc.resid.slice(0, 12)}`);
        if (rpc.cont === true) parts.push("cont");
        if (typeof rpc.error === "string") parts.push("error");
    } else {
        parts.push("kind=unknown");
    }
    parts.push(`size=${size}`);
    return parts.join(" ");
}

interface Clock {
    now: () => number;
    /** Runs `fn` after the microtasks already queued, the listener's promise continuations among them. */
    afterMicrotasks: (fn: () => void) => void;
    log: (line: string) => void;
}

const realClock: Clock = {
    now: () => performance.now(),
    afterMicrotasks: (fn) => queueMicrotask(fn),
    log: (line) => console.warn(line),
};

/**
 * Wraps the handling of one message: `watch(start, msg, size)` is called with the time the listener began and, once
 * the listener and the microtasks it queued are done, logs the message if it took long. Rate-limited.
 */
export function createSlowHandlerWatch(clock: Clock = realClock) {
    let windowStart = Number.NEGATIVE_INFINITY;
    let logged = 0;
    let skipped = 0;
    return function done(start: number, msg: unknown, size: number): void {
        const syncMs = clock.now() - start;
        clock.afterMicrotasks(() => {
            const total = clock.now() - start;
            if (total < SLOW_WS_HANDLER_MS) return;
            const t = clock.now();
            if (t - windowStart > SLOW_WS_LOG_WINDOW_MS) {
                windowStart = t;
                logged = 0;
            }
            if (logged >= SLOW_WS_LOG_MAX) {
                skipped++;
                return;
            }
            logged++;
            const more = skipped > 0 ? ` (+${skipped} slow messages not logged)` : "";
            skipped = 0;
            clock.log(
                `[perf] slow-ws-handler total=${Math.round(total)}ms sync=${Math.round(syncMs)}ms ${describeWsMessage(msg, size)}${more}`
            );
        });
    };
}
