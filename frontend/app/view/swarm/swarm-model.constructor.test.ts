// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What `new SwarmViewModel(blockId)` wires up: one subscription per event it
 * listens to, which reload each event triggers (and after what delay), what the
 * progress event stores, the safety-net poll, and what `dispose` tears down.
 *
 * Mocking follows swarm-model-backlog.test.ts: `mps` is module-mocked so
 * handlers are captured synchronously (and each unsubscribe is recorded),
 * `callBackendService` is spied on, and the RPC calls the model makes are
 * replaced with counters.
 */

import { WpsEvent } from "@/app/store/mps-events";
import * as mos from "@/store/mos";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
    subscribed: [] as string[],
    unsubscribed: [] as string[],
}));

const rpc = vi.hoisted(() => ({
    tracked: vi.fn(),
    background: vi.fn(),
    other: vi.fn(),
}));

vi.mock("@/app/store/mps", () => {
    const mock = {
        muxEventSubscribe: vi.fn((sub: { eventType: string; scope?: string; handler: (e: unknown) => void }) => {
            hub.subscribed.push(sub.eventType);
            hub.handlers.set(sub.eventType, sub.handler);
            return () => {
                hub.unsubscribed.push(sub.eventType);
                hub.handlers.delete(sub.eventType);
            };
        }),
    };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        mock.muxEventSubscribe({
            eventType,
            scope,
            handler: (e: unknown) => {
                const data = (e as { data?: unknown } | undefined)?.data;
                if (data != null) handler(data, e);
            },
        });
    return { ...mock, onMuxEvent };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: new Proxy(
        {},
        {
            get: (_t, name) => {
                if (name === "AgentTrackedBlocksCommand") return rpc.tracked;
                if (name === "ListBackgroundTasksCommand") return rpc.background;
                if (name === "SwarmOtherInstancesCommand") return rpc.other;
                return async () => ({});
            },
        }
    ),
}));

const callBackendServiceSpy = vi.spyOn(mos, "callBackendService").mockImplementation(async () => []);

import { SwarmViewModel } from "./swarm-model";

const SUBAGENT_EVENTS = [
    WpsEvent.SubagentSpawned,
    WpsEvent.SubagentCompleted,
    WpsEvent.SubagentUpdated,
    WpsEvent.SubagentAbandoned,
    WpsEvent.DispatchUpdated,
    WpsEvent.SubagentBlockPruned,
];

const TRACKED_BLOCK_EVENTS = [
    WpsEvent.AgentProcessAdded,
    WpsEvent.AgentProcessExited,
    WpsEvent.AgentReactiveRegistered,
    WpsEvent.AgentReactiveUnregistered,
    WpsEvent.ProcessBrokerTrackedBlocksChanged,
];

const ALL_EVENTS = [
    ...SUBAGENT_EVENTS,
    WpsEvent.ShellNodeCreate,
    WpsEvent.ShellChunk,
    WpsEvent.CronChanged,
    WpsEvent.BackgroundTaskUpdated,
    WpsEvent.AgentProgress,
    WpsEvent.SubagentNamed,
    ...TRACKED_BLOCK_EVENTS,
];

function fire(eventType: string, data: unknown = {}) {
    const handler = hub.handlers.get(eventType);
    expect(handler, `a handler for ${eventType}`).toBeDefined();
    handler!({ data });
}

function backendCalls(service: string, method: string): number {
    return callBackendServiceSpy.mock.calls.filter((c) => c[0] === service && c[1] === method).length;
}

/** The reloads the model can make, as call counts. */
function reloads() {
    return {
        subagents: backendCalls("subagent", "ListActive"),
        dispatches: backendCalls("subagent", "ListDispatches"),
        shells: backendCalls("shell", "ListActive"),
        crons: backendCalls("cron", "ListActive"),
        background: rpc.background.mock.calls.length,
        tracked: rpc.tracked.mock.calls.length,
        other: rpc.other.mock.calls.length,
    };
}

const NONE = { subagents: 0, dispatches: 0, shells: 0, crons: 0, background: 0, tracked: 0, other: 0 };

describe("SwarmViewModel constructor wiring", () => {
    let vm: SwarmViewModel;
    let initial: ReturnType<typeof reloads>;

    beforeEach(async () => {
        vi.useFakeTimers();
        hub.handlers.clear();
        hub.subscribed.length = 0;
        hub.unsubscribed.length = 0;
        rpc.tracked.mockReset().mockResolvedValue({ block_ids: [] });
        rpc.background.mockReset().mockResolvedValue([]);
        rpc.other.mockReset().mockResolvedValue(null);
        callBackendServiceSpy.mockClear();
        vm = new SwarmViewModel("block-1");
        // The constructor starts loadAll(); let it settle, then count only what the test causes.
        await vi.advanceTimersByTimeAsync(0);
        initial = reloads();
        callBackendServiceSpy.mockClear();
        rpc.tracked.mockClear();
        rpc.background.mockClear();
        rpc.other.mockClear();
    });

    afterEach(() => {
        vm.dispose();
        vi.useRealTimers();
    });

    it("keeps the block id it was given", () => {
        expect(vm.blockId).toBe("block-1");
    });

    it("loads every list once on construction", () => {
        expect(initial).toEqual({
            subagents: 1,
            dispatches: 1,
            shells: 1,
            crons: 1,
            background: 1,
            tracked: 1,
            other: 1,
        });
    });

    it("subscribes to each of its events exactly once", () => {
        expect([...hub.subscribed].sort()).toEqual([...ALL_EVENTS].sort());
    });

    describe("subagent and dispatch events", () => {
        it.each(SUBAGENT_EVENTS)(
            "%s reloads subagents and dispatches after 150ms, and nothing else",
            async (eventType) => {
                fire(eventType);
                await vi.advanceTimersByTimeAsync(149);
                expect(reloads()).toEqual(NONE);

                await vi.advanceTimersByTimeAsync(1);
                expect(reloads()).toEqual({ ...NONE, subagents: 1, dispatches: 1 });
            }
        );

        it("a burst of different subagent events is one reload", async () => {
            for (const eventType of SUBAGENT_EVENTS) fire(eventType);
            await vi.advanceTimersByTimeAsync(150);
            expect(reloads()).toEqual({ ...NONE, subagents: 1, dispatches: 1 });
        });
    });

    describe("shell events", () => {
        it("a created shell reloads the shells after 150ms, and nothing else", async () => {
            fire(WpsEvent.ShellNodeCreate);
            await vi.advanceTimersByTimeAsync(149);
            expect(reloads()).toEqual(NONE);

            await vi.advanceTimersByTimeAsync(1);
            expect(reloads()).toEqual({ ...NONE, shells: 1 });
        });

        it("a shell chunk reloads the shells only when it is the exit", async () => {
            fire(WpsEvent.ShellChunk, { op: "append" });
            await vi.advanceTimersByTimeAsync(500);
            expect(reloads()).toEqual(NONE);

            fire(WpsEvent.ShellChunk, { op: "exit" });
            await vi.advanceTimersByTimeAsync(149);
            expect(reloads()).toEqual(NONE);
            await vi.advanceTimersByTimeAsync(1);
            expect(reloads()).toEqual({ ...NONE, shells: 1 });
        });
    });

    it("a cron change reloads the crons after 150ms, and nothing else", async () => {
        fire(WpsEvent.CronChanged);
        await vi.advanceTimersByTimeAsync(149);
        expect(reloads()).toEqual(NONE);

        await vi.advanceTimersByTimeAsync(1);
        expect(reloads()).toEqual({ ...NONE, crons: 1 });
    });

    it("a background task update reloads the background tasks after 150ms, and nothing else", async () => {
        fire(WpsEvent.BackgroundTaskUpdated);
        await vi.advanceTimersByTimeAsync(149);
        expect(reloads()).toEqual(NONE);

        await vi.advanceTimersByTimeAsync(1);
        expect(reloads()).toEqual({ ...NONE, background: 1 });
    });

    describe("progress events", () => {
        it("store the block's checklist, dropping empty items, and its in-flight tool", () => {
            fire(WpsEvent.AgentProgress, {
                blockId: "b1",
                todos: [{ text: "write tests" }, { text: "" }, { text: "split it" }],
                todosTruncated: 3,
                todosPartial: false,
                currentTool: "Bash",
            });

            expect(vm.progressAtom().get("b1")).toEqual({
                todos: [{ text: "write tests" }, { text: "split it" }],
                todosTruncated: 3,
                todosPartial: false,
                currentTool: "Bash",
            });
        });

        it("keep the two flags apart", () => {
            fire(WpsEvent.AgentProgress, {
                blockId: "b1",
                todos: [],
                todosTruncated: 0,
                todosPartial: true,
                currentTool: null,
            });

            const stored = vm.progressAtom().get("b1");
            expect(stored?.todosTruncated).toBe(0);
            expect(stored?.todosPartial).toBe(true);
        });

        it("without a block id are ignored", () => {
            fire(WpsEvent.AgentProgress, { todos: [{ text: "x" }], todosTruncated: 0, todosPartial: false });
            fire(WpsEvent.AgentProgress, {
                blockId: "",
                todos: [{ text: "x" }],
                todosTruncated: 0,
                todosPartial: false,
            });

            expect(vm.progressAtom().size).toBe(0);
        });
    });

    describe("tracked-block events", () => {
        it.each(TRACKED_BLOCK_EVENTS)("%s re-reads the tracked blocks at once, and nothing else", async (eventType) => {
            fire(eventType);
            await vi.advanceTimersByTimeAsync(0);

            expect(reloads()).toEqual({ ...NONE, tracked: 1 });
        });
    });

    describe("safety-net poll", () => {
        it("re-reads the tracked blocks and the other instances every 12 seconds", async () => {
            await vi.advanceTimersByTimeAsync(11_999);
            expect(reloads()).toEqual(NONE);

            await vi.advanceTimersByTimeAsync(1);
            expect(reloads()).toEqual({ ...NONE, tracked: 1, other: 1 });

            await vi.advanceTimersByTimeAsync(12_000);
            expect(reloads()).toEqual({ ...NONE, tracked: 2, other: 2 });
        });
    });

    describe("dispose", () => {
        it("unsubscribes from every event", () => {
            vm.dispose();

            expect([...hub.unsubscribed].sort()).toEqual([...ALL_EVENTS].sort());
        });

        it("stops the poll", async () => {
            vm.dispose();
            await vi.advanceTimersByTimeAsync(36_000);

            expect(reloads()).toEqual(NONE);
        });

        it("cancels a reload that was waiting out its delay", async () => {
            fire(WpsEvent.SubagentSpawned);
            fire(WpsEvent.ShellNodeCreate);
            fire(WpsEvent.CronChanged);
            fire(WpsEvent.BackgroundTaskUpdated);
            vm.dispose();
            await vi.advanceTimersByTimeAsync(500);

            expect(reloads()).toEqual(NONE);
        });
    });
});
