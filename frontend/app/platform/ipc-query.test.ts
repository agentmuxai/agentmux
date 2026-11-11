// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invokeCommand } from "./ipc";
import { invokeFastCommand, queryLaneAvailable, resetQueryLane } from "./ipc-query";

vi.mock("./ipc", () => ({ invokeCommand: vi.fn(async () => "over-http") }));
vi.mock("@/perf", () => ({ recordIpcRoundtrip: vi.fn() }));

type Query = NonNullable<typeof window.agentmuxQuery>;

/** A host that answers every query one way. */
function host(answer: (q: Parameters<Query>[0]) => void): ReturnType<typeof vi.fn<Query>> {
    const fn = vi.fn<Query>((q) => {
        answer(q);
        return 1;
    });
    window.agentmuxQuery = fn;
    return fn;
}

beforeEach(() => {
    resetQueryLane();
    window.__AGENTMUX_IPC_TOKEN__ = "tok";
    vi.mocked(invokeCommand).mockClear();
});

afterEach(() => {
    delete window.agentmuxQuery;
});

describe("invokeFastCommand", () => {
    it("goes over HTTP when the window has no lane", async () => {
        delete window.agentmuxQuery;
        expect(queryLaneAvailable()).toBe(false);
        expect(await invokeFastCommand("browser_panes_set_rects", { a: 1 })).toBe("over-http");
        expect(invokeCommand).toHaveBeenCalledWith("browser_panes_set_rects", { a: 1 });
    });

    it("sends the command, the token and the args as one query and parses the answer", async () => {
        const fn = host((q) => q.onSuccess("true"));
        expect(queryLaneAvailable()).toBe(true);
        expect(await invokeFastCommand("browser_panes_set_rects", { window_label: "main", rects: [] })).toBe(true);
        expect(JSON.parse(fn.mock.calls[0][0].request)).toEqual({
            cmd: "browser_panes_set_rects",
            token: "tok",
            args: { window_label: "main", rects: [] },
        });
        expect(fn.mock.calls[0][0].persistent).toBe(false);
        expect(invokeCommand).not.toHaveBeenCalled();
    });

    it("falls back to HTTP when the host has no handler, and stops asking", async () => {
        const fn = host((q) => q.onFailure(-1, "canceled"));
        expect(await invokeFastCommand("browser_panes_set_rects", {})).toBe("over-http");
        expect(await invokeFastCommand("browser_panes_set_rects", {})).toBe("over-http");
        expect(fn).toHaveBeenCalledTimes(1);
        expect(invokeCommand).toHaveBeenCalledTimes(2);
        expect(queryLaneAvailable()).toBe(false);
    });

    it("falls back to HTTP for a refused query but tries the lane again next time", async () => {
        const fn = host((q) => q.onFailure(401, "the query's token is not this host's"));
        expect(await invokeFastCommand("browser_panes_set_rects", {})).toBe("over-http");
        expect(await invokeFastCommand("browser_panes_set_rects", {})).toBe("over-http");
        expect(fn).toHaveBeenCalledTimes(2);
        expect(invokeCommand).toHaveBeenCalledTimes(2);
    });

    it("goes over HTTP while the token is not injected yet", async () => {
        const fn = host((q) => q.onSuccess("true"));
        delete window.__AGENTMUX_IPC_TOKEN__;
        expect(await invokeFastCommand("browser_panes_set_rects", {})).toBe("over-http");
        expect(fn).not.toHaveBeenCalled();
    });
});
