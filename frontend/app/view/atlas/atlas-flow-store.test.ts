// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The lines on show follow srv: an event adds one, the recent lists fill in the last minutes, time takes them away. */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const bus = vi.hoisted(() => ({ handlers: new Map<string, (data: unknown) => void>(), off: vi.fn() }));
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (event: string, handler: (data: unknown) => void) => {
        bus.handlers.set(event, handler);
        return bus.off;
    },
}));
vi.mock("@/app/store/auth-headers", () => ({ authHeaders: () => ({ "X-AuthKey": "k" }) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));

import { LEAVING_MS, MESSAGE_SHOWN_MS } from "./atlas-flow";
import { useAtlasFlows } from "./atlas-flow-store";

const NOW = 1_000_000;
const message = (id: string, atMs: number, to = "bo") => ({
    msg_id: id,
    at_ms: atMs,
    from: { agent: "ann", host: "desk" },
    to: { agent: to, host: "desk" },
    via: "host",
    outcome: "delivered",
});
const task = (number: number, atMs: number) => ({
    task: { number, board: "main", host: "desk" },
    at_ms: atMs,
    by: { agent: "ann", host: "desk" },
    to: { agent: "bo", host: "desk" },
    kind: "filed",
});

/** srv's two recent lists, as its routes answer them. */
function serve(messages: unknown[], events: unknown[]) {
    const fetched = vi.fn(async (url: string) => ({
        ok: true,
        json: async () => (url.endsWith("/message-flow") ? { messages } : { events }),
    }));
    vi.stubGlobal("fetch", fetched);
    return fetched;
}

function mount() {
    return createRoot((dispose) => ({ flows: useAtlasFlows(), dispose }));
}

beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    bus.handlers.clear();
    bus.off.mockClear();
});

afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

describe("useAtlasFlows", () => {
    it("starts from what srv handled just before, leaving out what is too old to show", async () => {
        const fetched = serve([message("old", NOW - MESSAGE_SHOWN_MS - 1), message("recent", NOW - 2_000, "cy")], [task(12, NOW - 60_000)]);
        const { flows, dispose } = mount();
        await vi.advanceTimersByTimeAsync(0);
        expect(fetched).toHaveBeenCalledWith("http://srv/api/v1/message-flow", { headers: { "X-AuthKey": "k" } });
        expect(flows().map((f) => `${f.kind}:${f.label ?? f.to.agent}`)).toEqual(["message:cy", "task:#12"]);
        dispose();
    });

    it("adds a line when srv reports a message, timed from now, and takes it away when its time is up", async () => {
        serve([], []);
        const { flows, dispose } = mount();
        await vi.advanceTimersByTimeAsync(0);
        bus.handlers.get("message-flow")!(message("m1", NOW - 90_000));
        expect(flows()).toHaveLength(1);
        expect(flows()[0].leaving).toBe(false);
        await vi.advanceTimersByTimeAsync(MESSAGE_SHOWN_MS - LEAVING_MS + 500);
        expect(flows()[0].leaving).toBe(true);
        await vi.advanceTimersByTimeAsync(LEAVING_MS);
        expect(flows()).toEqual([]);
        expect(vi.getTimerCount()).toBe(0);
        dispose();
    });

    it("adds a task hand-over from its event", async () => {
        serve([], []);
        const { flows, dispose } = mount();
        await vi.advanceTimersByTimeAsync(0);
        bus.handlers.get("task-flow")!(task(7, NOW));
        expect(flows().map((f) => f.label)).toEqual(["#7"]);
        dispose();
    });

    it("shows nothing, and does not fail, against an srv with no such route", async () => {
        vi.stubGlobal("fetch", vi.fn(async () => ({ ok: false, json: async () => ({}) })));
        const { flows, dispose } = mount();
        await vi.advanceTimersByTimeAsync(0);
        expect(flows()).toEqual([]);
        dispose();
    });

    it("stops listening and keeping time when the pane goes", async () => {
        serve([], []);
        const { dispose } = mount();
        await vi.advanceTimersByTimeAsync(0);
        bus.handlers.get("message-flow")!(message("m1", NOW));
        dispose();
        expect(bus.off).toHaveBeenCalledTimes(2);
        expect(vi.getTimerCount()).toBe(0);
    });
});
