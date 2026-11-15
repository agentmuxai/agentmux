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

import { acceptMuxbusDelivery, RECONNECTING_QUIET_MS } from "@/app/store/muxbus-delivery";
import { LEAVING_MS, MESSAGE_SHOWN_MS, PR_FRESH_MS } from "./atlas-flow";
import { useAtlasFlows, useCloudPaused } from "./atlas-flow-store";

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

const review = (id: string, atMs: number, number = 64, state = "approved") => ({
    id,
    at_ms: atMs,
    agent: "ann",
    pr: { repo_id: 9, number },
    state,
    by: "bot",
});
const checks = (id: string, atMs: number, number = 64, state = "passing") => ({ id, at_ms: atMs, agent: "ann", pr: { repo_id: 9, number }, state });
const merge = (id: string, atMs: number, number = 64) => ({ id, at_ms: atMs, agent: "ann", pr: { repo_id: 9, number }, state: "merged" });

/** srv's recent lists, as its routes answer them. */
function serve(messages: unknown[], events: unknown[], pr: unknown = {}) {
    const fetched = vi.fn(async (url: string) => ({
        ok: true,
        json: async () => (url.endsWith("/message-flow") ? { messages } : url.endsWith("/pr-flow") ? pr : { events }),
    }));
    vi.stubGlobal("fetch", fetched);
    return fetched;
}

function mount() {
    return createRoot((dispose) => {
        const live = useAtlasFlows();
        return { flows: live.flows, lastPrNoticeAtMs: live.lastPrNoticeAtMs, dispose };
    });
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
        expect(flows().map((f) => `${f.kind}:${f.label ?? ("agent" in f.to ? f.to.agent : "")}`)).toEqual(["message:cy", "task:#12"]);
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
        expect(bus.off).toHaveBeenCalledTimes(5);
        expect(vi.getTimerCount()).toBe(0);
    });
});

describe("a pull request's review and checks", () => {
    it("start from srv's recent list: dim when past their first minutes, and nothing for a pull request that has merged", async () => {
        const fetched = serve([], [], {
            review: [review("r1", NOW - PR_FRESH_MS - 1), review("r2", NOW - 1_000, 65), review("r3", NOW - 1_000, 66)],
            checks: [checks("c1", NOW - 1_000, 65, "failing")],
            merge: [merge("m1", NOW - 500, 66)],
            bot: { last_notice_at_ms: NOW - 500 },
        });
        const { flows, lastPrNoticeAtMs, dispose } = mount();
        await vi.advanceTimersByTimeAsync(0);
        expect(fetched).toHaveBeenCalledWith("http://srv/api/v1/pr-flow", { headers: { "X-AuthKey": "k" } });
        expect(flows().map((f) => `${f.kind} ${f.label} ${f.tone}${f.dim ? " dim" : ""}`)).toEqual([
            "review #64 passed dim",
            "review #65 passed",
            "checks #65 failed",
        ]);
        expect(lastPrNoticeAtMs()).toBe(NOW - 500);
        dispose();
    });

    it("a new review replaces the one before for the same pull request, and its merge takes its lines away", async () => {
        serve([], []);
        const { flows, lastPrNoticeAtMs, dispose } = mount();
        await vi.advanceTimersByTimeAsync(0);
        expect(lastPrNoticeAtMs()).toBeNull();
        bus.handlers.get("review-flow")!(review("r1", NOW, 64, "changes_requested"));
        bus.handlers.get("checks-flow")!(checks("c1", NOW));
        bus.handlers.get("review-flow")!(review("r2", NOW + 10));
        bus.handlers.get("review-flow")!(review("r3", NOW + 20, 65));
        expect(flows().map((f) => `${f.id} ${f.state}`)).toEqual(["checks:c1 passing", "review:r2 approved", "review:r3 approved"]);
        expect(lastPrNoticeAtMs()).toBe(NOW + 20);
        bus.handlers.get("merge-flow")!(merge("m1", NOW + 30));
        expect(flows().map((f) => f.id)).toEqual(["review:r3"]);
        // A notice about the merged pull request that comes late draws nothing.
        bus.handlers.get("checks-flow")!(checks("c2", NOW + 40));
        expect(flows().map((f) => f.id)).toEqual(["review:r3"]);
        expect(lastPrNoticeAtMs()).toBe(NOW + 40);
        dispose();
    });

    it("stay, dimmed, long after a message's line has gone", async () => {
        serve([], []);
        const { flows, dispose } = mount();
        await vi.advanceTimersByTimeAsync(0);
        bus.handlers.get("review-flow")!(review("r1", NOW));
        bus.handlers.get("message-flow")!(message("m1", NOW));
        await vi.advanceTimersByTimeAsync(PR_FRESH_MS + 1_000);
        expect(flows().map((f) => `${f.kind}${f.dim ? " dim" : ""}`)).toEqual(["review dim"]);
        dispose();
    });
});

describe("useCloudPaused", () => {
    const status = (state: string, sinceMs: number) => ({ state, since_ms: sinceMs });

    it("is so while MuxBus needs a sign-in, and after it has been out of reach for longer than a passing reconnect", async () => {
        acceptMuxbusDelivery(status("connected", NOW));
        const { paused, dispose } = createRoot((d) => ({ paused: useCloudPaused(), dispose: d }));
        await vi.advanceTimersByTimeAsync(0);
        expect(paused()).toBe(false);
        acceptMuxbusDelivery(status("needs_sign_in", NOW));
        expect(paused()).toBe(true);
        acceptMuxbusDelivery(status("reconnecting", NOW));
        await vi.advanceTimersByTimeAsync(RECONNECTING_QUIET_MS - 1_000);
        expect(paused()).toBe(false);
        await vi.advanceTimersByTimeAsync(1_000);
        expect(paused()).toBe(true);
        acceptMuxbusDelivery(status("connected", NOW));
        await vi.advanceTimersByTimeAsync(0);
        expect(paused()).toBe(false);
        dispose();
        expect(vi.getTimerCount()).toBe(0);
    });
});
