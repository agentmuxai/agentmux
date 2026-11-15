// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    SLOW_WS_HANDLER_MS,
    SLOW_WS_LOG_MAX,
    SLOW_WS_LOG_WINDOW_MS,
    createSlowHandlerWatch,
    describeWsMessage,
} from "./ws-slow-handler";

const event = (name: string, scopes: string[], data?: unknown) => ({
    eventtype: "rpc",
    data: { command: "eventrecv", data: { event: name, scopes, data } },
});

describe("describeWsMessage", () => {
    it("names an event, the block it is scoped to, and the items it carries", () => {
        const line = describeWsMessage(event("objects:updated", ["workspace:w1", "block:abcdef0123456789"], [1, 2, 3]), 9000);
        expect(line).toBe("kind=event:objects:updated block=block:abcdef0123456789 items=3 size=9000");
    });

    it("counts the updates of an event that carries them in a list field", () => {
        const line = describeWsMessage(event("objects:updated", [], { updates: [{}, {}] }), 10);
        expect(line).toContain("items=2");
    });

    it("names an RPC request by its command, and a response by its id", () => {
        expect(describeWsMessage({ eventtype: "rpc", data: { command: "getfullconfig", reqid: "r-1" } }, 5)).toBe(
            "kind=rpc:getfullconfig reqid=r-1 size=5"
        );
        expect(describeWsMessage({ eventtype: "rpc", data: { resid: "r-1", cont: true } }, 7)).toBe(
            "kind=rpc-response resid=r-1 cont size=7"
        );
    });

    it("says unknown for a message it cannot place, and never throws on junk", () => {
        expect(describeWsMessage({ eventtype: "rpc", data: {} }, 1)).toBe("kind=unknown size=1");
        expect(describeWsMessage(null, 0)).toBe("kind=unknown size=0");
        expect(describeWsMessage("text", 4)).toBe("kind=unknown size=4");
    });

    it("carries none of the message's content", () => {
        const line = describeWsMessage(event("x", ["block:b1"], { secret: "hunter2" }), 30);
        expect(line).not.toContain("hunter2");
    });
});

/** A clock the test moves, and microtasks it runs by hand. */
function fakeClock() {
    let t = 1000;
    const queued: (() => void)[] = [];
    const lines: string[] = [];
    return {
        clock: {
            now: () => t,
            afterMicrotasks: (fn: () => void) => queued.push(fn),
            log: (line: string) => lines.push(line),
        },
        advance: (ms: number) => (t += ms),
        runMicrotasks: () => queued.splice(0).forEach((fn) => fn()),
        lines,
    };
}

describe("createSlowHandlerWatch", () => {
    it("logs a message handled in 200 ms or more, with its sync time and total", () => {
        const f = fakeClock();
        const watch = createSlowHandlerWatch(f.clock);
        const start = f.clock.now();
        f.advance(120); // the listener
        watch(start, event("objects:updated", ["block:b1"]), 500);
        f.advance(200); // what resumed after it, in the same task
        f.runMicrotasks();
        expect(f.lines).toEqual([
            "[perf] slow-ws-handler total=320ms sync=120ms kind=event:objects:updated block=block:b1 size=500",
        ]);
    });

    it("says nothing for a message under the threshold", () => {
        const f = fakeClock();
        const watch = createSlowHandlerWatch(f.clock);
        const start = f.clock.now();
        f.advance(SLOW_WS_HANDLER_MS - 1);
        watch(start, event("x", []), 1);
        f.runMicrotasks();
        expect(f.lines).toEqual([]);
    });

    it("counts a 28 s message as one line, not a flood", () => {
        const f = fakeClock();
        const watch = createSlowHandlerWatch(f.clock);
        const start = f.clock.now();
        f.advance(27_870);
        watch(start, event("objects:updated", []), 1);
        f.runMicrotasks();
        expect(f.lines).toHaveLength(1);
        expect(f.lines[0]).toContain("total=27870ms");
    });

    it("logs at most SLOW_WS_LOG_MAX lines in a window, and says how many it left out in the next", () => {
        const f = fakeClock();
        const watch = createSlowHandlerWatch(f.clock);
        const slow = () => {
            const start = f.clock.now();
            f.advance(SLOW_WS_HANDLER_MS);
            watch(start, event("x", []), 1);
            f.runMicrotasks();
        };
        for (let i = 0; i < SLOW_WS_LOG_MAX + 5; i++) slow();
        expect(f.lines).toHaveLength(SLOW_WS_LOG_MAX);
        f.advance(SLOW_WS_LOG_WINDOW_MS);
        slow();
        expect(f.lines).toHaveLength(SLOW_WS_LOG_MAX + 1);
        expect(f.lines.at(-1)).toContain("(+5 slow messages not logged)");
    });
});
