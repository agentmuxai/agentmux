// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Names the code that makes the page lay out inside a websocket message's handling. The `[perf] long-frame` line for
 * `DOMWebSocket.onmessage` carries the forced layout's time and nothing about who asked for it. With the
 * `debug:layoutreadprobe` setting on (off by default), the calls that make the browser lay out so they can answer
 * (`getBoundingClientRect`, `offsetWidth`, `clientHeight`, `scrollTop` and the like) are timed while a message is being
 * handled, and one that took `READ_MIN_MS` or more writes a `[perf] layout-read` line: the message's kind, the call, the
 * place in the code that made it, and the milliseconds. Never message content: the kind is the event's or command's name
 * (`ws-handler-cost.ts`), and the place is a script address and line from the call's stack.
 *
 * It is a measuring aid for one question, so it is bounded: a place is written at most once in `REPEAT_MS` (the line
 * after says how many it missed and the worst), and no more than `MAX_LINES` in a window.
 */

import { wsMessageKind } from "./ws-handler-cost";

/** A read that took this long, or longer, is written. A read that finds the layout current costs far less. */
export const READ_MIN_MS = 1;
/** The same kind, call and place is written at most once in this long. */
export const REPEAT_MS = 10_000;
/** At most this many lines in a window, so a stretch of slow reads does not make the log pipe part of the problem. */
export const MAX_LINES = 20;
export const LINES_WINDOW_MS = 10_000;
/** More distinct places than this and the rest are not tracked, so the table stays bounded. */
export const MAX_PLACES = 64;

interface Place {
    lastWritten: number;
    missed: number;
    worstMs: number;
}

interface Clock {
    now: () => number;
    log: (line: string) => void;
    /** The stack of the call being timed; read only for a slow one. */
    stack: () => string;
}

const realClock: Clock = {
    now: () => performance.now(),
    log: (line) => console.warn(line),
    stack: () => new Error().stack ?? "",
};

/**
 * The first place in a stack that is not this file: `<script path>:<line>:<column>`, without the origin or a query. A
 * packaged build puts this file and its callers in one bundle, so the stack handed to it is cut at the wrapper
 * (`stackAbove`) and this name filter only covers a development build's own frames.
 */
export function callerOf(stack: string): string {
    for (const line of stack.split("\n").slice(1)) {
        if (/layout-read-probe\.ts\b/.test(line)) continue;
        const at = /\(?([^\s()]+):(\d+):(\d+)\)?\s*$/.exec(line.trim());
        if (at == null) continue;
        const path = at[1].replace(/^[a-z][a-z0-9+.-]*:\/\/[^/]*/i, "").replace(/[?#].*$/, "");
        return `${path.slice(-80)}:${at[2]}:${at[3]}`;
    }
    return "?";
}

export function createLayoutReadProbe(clock: Clock = realClock) {
    let enabled = false;
    let kind: string | null = null;
    let places = new Map<string, Place>();
    let linesAt = clock.now();
    let lines = 0;

    function write(api: string, ms: number, stack?: () => string): void {
        const at = callerOf((stack ?? clock.stack)());
        const key = `${kind}|${api}|${at}`;
        const now = clock.now();
        let place = places.get(key);
        if (place == null) {
            if (places.size >= MAX_PLACES) return;
            place = { lastWritten: -Infinity, missed: 0, worstMs: 0 };
            places.set(key, place);
        }
        place.worstMs = Math.max(place.worstMs, ms);
        if (now - place.lastWritten < REPEAT_MS) {
            place.missed++;
            return;
        }
        if (now - linesAt >= LINES_WINDOW_MS) {
            linesAt = now;
            lines = 0;
        }
        if (lines >= MAX_LINES) {
            place.missed++;
            return;
        }
        lines++;
        const more = place.missed > 0 ? ` missed=${place.missed} worst=${place.worstMs.toFixed(1)}ms` : "";
        clock.log(`[perf] layout-read kind=${kind} api=${api} at=${at} ms=${ms.toFixed(1)}${more}`);
        place.lastWritten = now;
        place.missed = 0;
        place.worstMs = 0;
    }

    return {
        setEnabled(on: boolean): void {
            enabled = on;
            if (!on) {
                kind = null;
                places = new Map();
            }
        },
        /** Before a message's handlers run. */
        enter(msg: unknown): void {
            if (enabled) kind = wsMessageKind(msg);
        },
        /** After them. */
        leave(): void {
            kind = null;
        },
        /** Whether a read now is inside a message's handling. */
        active: (): boolean => kind !== null,
        /** The browser's answer to a layout-reading call took `ms`. */
        read(api: string, ms: number, stack?: () => string): void {
            if (kind !== null && ms >= READ_MIN_MS) write(api, ms, stack);
        },
        now: clock.now,
    };
}

export type LayoutReadProbe = ReturnType<typeof createLayoutReadProbe>;

export const layoutReadProbe = createLayoutReadProbe();

interface Wrapped {
    proto: object;
    name: string;
    original: PropertyDescriptor;
}

const METHODS: [string, string][] = [
    ["Element", "getBoundingClientRect"],
    ["Element", "getClientRects"],
];
const GETTERS: [string, string][] = [
    ["HTMLElement", "offsetWidth"],
    ["HTMLElement", "offsetHeight"],
    ["HTMLElement", "offsetTop"],
    ["HTMLElement", "offsetLeft"],
    ["Element", "clientWidth"],
    ["Element", "clientHeight"],
    ["Element", "scrollWidth"],
    ["Element", "scrollHeight"],
    ["Element", "scrollTop"],
    ["Element", "scrollLeft"],
];

/** The stack at this call without the frames from the top down to `above`, where V8 can cut it; the whole stack elsewhere. */
function stackAbove(above: Function): string {
    const holder: { stack?: string } = {};
    (Error as { captureStackTrace?: (o: object, f: Function) => void }).captureStackTrace?.(holder, above);
    return holder.stack ?? new Error().stack ?? "";
}

/**
 * Times those calls on the page's elements for the probe. Returns what undoes it. A call outside a message's handling
 * pays one check of the probe's state.
 */
export function wrapLayoutReads(probe: LayoutReadProbe, scope: Record<string, any> = globalThis): () => void {
    const done: Wrapped[] = [];
    const timed = <T>(name: string, run: () => T, wrapper: Function): T => {
        if (!probe.active()) return run();
        const start = probe.now();
        try {
            return run();
        } finally {
            probe.read(name, probe.now() - start, () => stackAbove(wrapper));
        }
    };
    for (const [ctor, name] of METHODS) {
        const proto = scope[ctor]?.prototype;
        const original = proto && Object.getOwnPropertyDescriptor(proto, name);
        if (!original || typeof original.value !== "function") continue;
        const fn = original.value;
        const value = function (this: unknown, ...args: unknown[]) {
            return timed(name, () => fn.apply(this, args), value);
        };
        Object.defineProperty(proto, name, { ...original, value });
        done.push({ proto, name, original });
    }
    for (const [ctor, name] of GETTERS) {
        const proto = scope[ctor]?.prototype;
        const original = proto && Object.getOwnPropertyDescriptor(proto, name);
        if (!original || typeof original.get !== "function") continue;
        const get = original.get;
        const wrapped = function (this: unknown) {
            return timed(name, () => get.call(this), wrapped);
        };
        Object.defineProperty(proto, name, { ...original, get: wrapped });
        done.push({ proto, name, original });
    }
    return () => {
        for (const w of done) Object.defineProperty(w.proto, w.name, w.original);
    };
}
