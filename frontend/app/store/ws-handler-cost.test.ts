// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    COST_MAX_KINDS,
    COST_MIN_TOTAL_MS,
    COST_TOP_KINDS,
    COST_WINDOW_MS,
    createHandlerCostTally,
    wsMessageKind,
} from "./ws-handler-cost";

const event = (name: string) => ({ eventtype: "rpc", data: { command: "eventrecv", data: { event: name } } });

function harness() {
    let clock = 0;
    const lines: string[] = [];
    const note = createHandlerCostTally({ now: () => clock, log: (l) => lines.push(l) });
    /** One message that took `ms`, handled at the current clock. */
    const handle = (msg: unknown, ms: number) => {
        const start = clock;
        clock += ms;
        note(start, msg);
    };
    return { lines, handle, advance: (ms: number) => (clock += ms) };
}

describe("wsMessageKind", () => {
    it("names an event, an RPC request, a response, and anything else, without content", () => {
        expect(wsMessageKind(event("blockfile"))).toBe("event:blockfile");
        expect(wsMessageKind({ data: { command: "eventrecv", data: {} } })).toBe("event:?");
        expect(wsMessageKind({ data: { command: "getfullconfig" } })).toBe("rpc:getfullconfig");
        expect(wsMessageKind({ data: { resid: "r-1" } })).toBe("rpc-response");
        expect(wsMessageKind({ data: {} })).toBe("other");
        expect(wsMessageKind(null)).toBe("other");
        expect(wsMessageKind("junk")).toBe("other");
    });
});

describe("the handler cost tally", () => {
    it("writes nothing for a quiet window", () => {
        const { lines, handle, advance } = harness();
        for (let i = 0; i < 100; i++) handle(event("blockfile"), 1);
        advance(COST_WINDOW_MS);
        handle(event("blockfile"), 1);
        expect(lines).toEqual([]);
    });

    it("names the kinds the window's time went to, most expensive first, with count, total, mean and max", () => {
        const { lines, handle, advance } = harness();
        for (let i = 0; i < 100; i++) handle(event("blockfile"), 12);
        for (let i = 0; i < 50; i++) handle(event("sysinfo"), 1);
        handle(event("blockfile"), 40);
        advance(COST_WINDOW_MS);
        handle(event("sysinfo"), 1);
        expect(lines).toHaveLength(1);
        expect(lines[0]).toContain("[perf] ws-handler-cost");
        // The message that closes the window is counted in it: 101 + 50 + 1.
        expect(lines[0]).toContain("messages=152");
        expect(lines[0]).toMatch(/top: event:blockfile n=101 total=1240ms mean=12\.3ms max=40ms; event:sysinfo n=51 total=51ms/);
    });

    it("starts a new window after a line, so the next one counts only its own messages", () => {
        const { lines, handle, advance } = harness();
        for (let i = 0; i < 100; i++) handle(event("a"), 10);
        advance(COST_WINDOW_MS);
        handle(event("a"), 10);
        for (let i = 0; i < 100; i++) handle(event("b"), 10);
        advance(COST_WINDOW_MS);
        handle(event("b"), 10);
        expect(lines).toHaveLength(2);
        expect(lines[1]).toContain("top: event:b ");
        expect(lines[1]).not.toContain("event:a ");
    });

    it("names at most COST_TOP_KINDS kinds, and stays bounded however many kinds arrive", () => {
        const { lines, handle, advance } = harness();
        for (let i = 0; i < COST_MAX_KINDS + 30; i++) handle(event(`kind-${i}`), COST_MIN_TOTAL_MS);
        advance(COST_WINDOW_MS);
        handle(event("kind-0"), 1);
        const named = lines[0].split("top: ")[1].split("; ");
        expect(named).toHaveLength(COST_TOP_KINDS);
        expect(lines[0]).toContain(`messages=${COST_MAX_KINDS + 30 + 1}`);
    });
});
