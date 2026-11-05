// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useTurnLifecycle — characterisation of the paths the other test file leaves out: what
 * finalizeTurn records and dispatches (token usage by source, the agent name, the "Interrupted"
 * row and its once-only guard), the process-exit grace timer, the Esc stop-fallback timer, the
 * submit-ack timer's scope, and the watchdog interval. Written ahead of a split of the hook.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
    scopes: [] as { eventType: string; scope?: string }[],
    unsubscribed: [] as string[],
    snapshot: null as unknown,
    snapshotBlocks: [] as string[],
    blockObject: undefined as unknown,
    objectRefs: [] as string[],
    recorded: [] as { provider: string; tokens: unknown; meta: unknown }[],
}));

vi.mock("@/app/store/mps", () => {
    const muxEventSubscribe = vi.fn((sub: { eventType: string; scope?: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        hub.scopes.push({ eventType: sub.eventType, scope: sub.scope });
        return () => {
            hub.unsubscribed.push(sub.eventType);
            hub.handlers.delete(sub.eventType);
        };
    });
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        muxEventSubscribe({
            eventType,
            scope,
            handler: (e) => {
                const ev = e as { data?: unknown } | null;
                if (ev?.data != null) handler(ev.data, ev);
            },
        });
    return { muxEventSubscribe, onMuxEvent };
});
vi.mock("@/app/store/mos", () => ({
    makeORef: (a: string, b: string) => `${a}:${b}`,
    getObjectValue: (oref: string) => {
        hub.objectRefs.push(oref);
        return hub.blockObject;
    },
}));
vi.mock("@/app/store/agent-pane-state-store", () => ({
    snapshot: (blockId: string) => {
        hub.snapshotBlocks.push(blockId);
        return hub.snapshot;
    },
}));
vi.mock("@/store/token-usage", () => ({
    recordTurn: (provider: string, tokens: unknown, meta: unknown) => hub.recorded.push({ provider, tokens, meta }),
}));

import { SUBMIT_TIMEOUT_MS, type TurnPhase } from "@/app/store/agent-pane-state/types";
import type { SessionStats } from "../../../agent-model/types";
import { useTurnLifecycle, type UseTurnLifecycleOptions } from "./useTurnLifecycle";

const T0 = 1_700_000_000_000;
const BLOCK = "abcdefghijkl";

const phase = (kind: string, extra: Record<string, unknown> = {}) => ({ kind, ...extra }) as unknown as TurnPhase;
const idle = () => phase("Idle");
const submitting = () => phase("Submitting", { submittedAt: T0, pendingContent: "" });
const streaming = () => phase("Streaming", { bufferSize: 0, toolsActive: 0, lastEventMs: T0 });
const interrupting = () => phase("Interrupting");

let dispose: (() => void) | undefined;

function mount(initial: TurnPhase = idle(), over: Partial<UseTurnLifecycleOptions> = {}) {
    const dispatchPane = vi.fn();
    const queue = { pushNewNode: vi.fn(), scheduleFlush: vi.fn() };
    const flushParserPending = vi.fn();
    const nodeIds = new Set<string>();
    const addNodeId = vi.fn((id: string) => nodeIds.add(id));
    const hasNodeId = vi.fn((id: string) => nodeIds.has(id));
    let setPhase!: (p: TurnPhase) => void;
    let result!: ReturnType<typeof useTurnLifecycle>;
    createRoot((d) => {
        dispose = d;
        const [getPhase, setP] = createSignal<TurnPhase>(initial);
        setPhase = setP;
        result = useTurnLifecycle({
            blockId: BLOCK,
            model: { dispatchPane } as unknown as UseTurnLifecycleOptions["model"],
            turnPhase: getPhase,
            queue: queue as unknown as UseTurnLifecycleOptions["queue"],
            flushParserPending,
            hasNodeId,
            addNodeId,
            ...over,
        });
    });
    const all = () => dispatchPane.mock.calls.map((c) => c[0] as { type: string; [k: string]: unknown });
    const dispatched = () => all().filter((d) => d.type !== "StreamWatchdogTick");
    const ticks = () => all().filter((d) => d.type === "StreamWatchdogTick");
    return { dispatchPane, queue, flushParserPending, addNodeId, hasNodeId, setPhase, result, dispatched, ticks };
}

const controllerStatus = (status: string) => {
    const h = hub.handlers.get("controllerstatus");
    if (!h) throw new Error("no controllerstatus handler");
    h({ data: { shellprocstatus: status } });
};

beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(T0);
    hub.handlers.clear();
    hub.scopes.length = 0;
    hub.unsubscribed.length = 0;
    hub.snapshot = null;
    hub.snapshotBlocks.length = 0;
    hub.blockObject = undefined;
    hub.objectRefs.length = 0;
    hub.recorded.length = 0;
});
afterEach(() => {
    dispose?.();
    dispose = undefined;
    vi.useRealTimers();
});

describe("finalizeTurn", () => {
    it("flushes the parser's pending text first, then ends the turn with the stats", () => {
        const order: string[] = [];
        const m = mount(idle(), { flushParserPending: () => order.push("flush") });
        m.dispatchPane.mockImplementation(() => order.push("TurnEnd"));
        const stats = { input_tokens: 1, output_tokens: 2 } as SessionStats;
        m.result.finalizeTurn(stats);
        expect(order).toEqual(["flush", "TurnEnd"]);
        expect(m.dispatched()).toEqual([{ type: "TurnEnd", stats }]);
        m.result.finalizeTurn(null);
        expect(m.dispatched().at(-1)).toEqual({ type: "TurnEnd", stats: null });
    });

    it("records a turn only for a pane that names its provider", () => {
        const m = mount(idle());
        m.result.finalizeTurn(null);
        expect(hub.recorded).toEqual([]);
    });

    it("records the result event's usage, with every part of the breakdown", () => {
        const m = mount(idle(), { provider: "claude" });
        m.result.finalizeTurn({
            input_tokens: 10,
            output_tokens: 20,
            fresh_input_tokens: 3,
            cache_creation_input_tokens: 4,
            cache_read_input_tokens: 5,
            cost_usd: 0.25,
            num_turns: 7,
        } as SessionStats);
        expect(hub.recorded).toHaveLength(1);
        expect(hub.recorded[0].provider).toBe("claude");
        expect(hub.recorded[0].tokens).toEqual({
            input: 10,
            output: 20,
            freshInput: 3,
            cacheCreation: 4,
            cacheRead: 5,
        });
        expect(hub.recorded[0].meta).toEqual({ blockId: BLOCK, agentName: "abcdefg", costUsd: 0.25, numTurns: 7 });
    });

    it("treats a missing half of the result usage as zero", () => {
        const m = mount(idle(), { provider: "claude" });
        m.result.finalizeTurn({ input_tokens: 10 } as SessionStats);
        expect(hub.recorded[0].tokens).toMatchObject({ input: 10, output: 0 });
        m.result.finalizeTurn({ output_tokens: 6 } as SessionStats);
        expect(hub.recorded[1].tokens).toMatchObject({ input: 0, output: 6 });
    });

    it("falls back to the live turn tokens when the result carries no usage", () => {
        hub.snapshot = {
            turnTokens: { input: 100, output: 7, outputDone: 5, freshInput: 11, cacheCreation: 12, cacheRead: 13 },
        };
        const m = mount(idle(), { provider: "claude" });
        m.result.finalizeTurn(null);
        expect(hub.snapshotBlocks).toContain(BLOCK);
        expect(hub.recorded[0].tokens).toEqual({
            input: 100,
            output: 12,
            freshInput: 11,
            cacheCreation: 12,
            cacheRead: 13,
        });
        m.result.finalizeTurn({ cost_usd: 0.5, num_turns: 2 } as SessionStats);
        expect(hub.recorded[1].tokens).toEqual({
            input: 100,
            output: 12,
            freshInput: 11,
            cacheCreation: 12,
            cacheRead: 13,
        });
        expect(hub.recorded[1].meta).toMatchObject({ costUsd: 0.5, numTurns: 2 });
    });

    it("prefers the result's usage over the live turn tokens", () => {
        hub.snapshot = { turnTokens: { input: 100, output: 7, freshInput: 1, cacheCreation: 2, cacheRead: 3 } };
        const m = mount(idle(), { provider: "claude" });
        m.result.finalizeTurn({ input_tokens: 1, output_tokens: 2 } as SessionStats);
        expect(hub.recorded[0].tokens).toMatchObject({ input: 1, output: 2 });
    });

    it("still records, with no tokens, when there is no usage at all", () => {
        const m = mount(idle(), { provider: "claude" });
        m.result.finalizeTurn({ cost_usd: 0.1 } as SessionStats);
        expect(hub.recorded).toHaveLength(1);
        expect(hub.recorded[0].tokens).toBeNull();
        expect(hub.recorded[0].meta).toMatchObject({ costUsd: 0.1, numTurns: undefined });
        m.result.finalizeTurn(null);
        expect(hub.recorded[1].tokens).toBeNull();
        expect(hub.recorded[1].meta).toMatchObject({ costUsd: undefined, numTurns: undefined });
    });

    it("records the usage before the turn's end clears the live tokens", () => {
        hub.snapshot = { turnTokens: { input: 7, output: 3, outputDone: 0 } };
        const m = mount(idle(), { provider: "claude" });
        m.dispatchPane.mockImplementation(() => {
            hub.snapshot = null;
        });
        m.result.finalizeTurn(null);
        expect(hub.recorded[0].tokens).toMatchObject({ input: 7, output: 3 });
    });

    it("counts the live turn's output as its total, not the last call's", () => {
        hub.snapshot = { turnTokens: { input: 1, output: 4, outputDone: 10, shownOutput: 20 } };
        const m = mount(idle(), { provider: "claude" });
        m.result.finalizeTurn(null);
        expect((hub.recorded[0].tokens as { output: number }).output).toBe(20);
    });

    it("names the agent from its block, or the first seven characters of the block id", () => {
        hub.blockObject = { meta: { agentName: "Agent9" } };
        const m = mount(idle(), { provider: "claude" });
        m.result.finalizeTurn(null);
        expect(hub.objectRefs).toEqual([`block:${BLOCK}`]);
        expect(hub.recorded[0].meta).toMatchObject({ agentName: "Agent9" });
        hub.blockObject = { meta: {} };
        m.result.finalizeTurn(null);
        expect(hub.recorded[1].meta).toMatchObject({ agentName: "abcdefg" });
    });

    it("adds no Interrupted row to a turn the user did not stop", () => {
        const m = mount(streaming());
        m.result.finalizeTurn(null);
        expect(m.queue.pushNewNode).not.toHaveBeenCalled();
        expect(m.queue.scheduleFlush).not.toHaveBeenCalled();
        expect(m.addNodeId).not.toHaveBeenCalled();
    });

    it("adds one Interrupted row to a turn the user stopped, through the shared queue", () => {
        const m = mount(interrupting());
        m.result.finalizeTurn(null);
        expect(m.queue.pushNewNode).toHaveBeenCalledTimes(1);
        expect(m.queue.pushNewNode).toHaveBeenCalledWith({
            type: "markdown",
            id: `interrupted-${T0}`,
            content: "⏹ _Interrupted by user_",
            timestamp: T0,
        });
        expect(m.addNodeId).toHaveBeenCalledWith(`interrupted-${T0}`);
        expect(m.queue.scheduleFlush).toHaveBeenCalledTimes(1);
    });

    it("decides it was a stop before the turn ends, and does not add the row twice in the same millisecond", () => {
        const m = mount(interrupting());
        const order: string[] = [];
        m.dispatchPane.mockImplementation(() => {
            order.push("TurnEnd");
            m.setPhase(idle());
        });
        m.queue.pushNewNode.mockImplementation(() => order.push("row"));
        m.result.finalizeTurn(null);
        expect(order).toEqual(["TurnEnd", "row"]);
        m.setPhase(interrupting());
        m.result.finalizeTurn(null);
        expect(m.queue.pushNewNode).toHaveBeenCalledTimes(1);
        expect(m.dispatched().filter((d) => d.type === "TurnEnd")).toHaveLength(2);
        vi.advanceTimersByTime(1);
        m.setPhase(interrupting());
        m.result.finalizeTurn(null);
        expect(m.queue.pushNewNode).toHaveBeenCalledTimes(2);
    });
});

describe("the process-exit grace timer", () => {
    it("listens to this block's controller status", () => {
        mount();
        expect(hub.scopes).toContainEqual({ eventType: "controllerstatus", scope: `block:${BLOCK}` });
    });

    it.each([["Streaming"], ["Submitting"]])(
        "after a clean exit finds the turn still %s 1.5 s later, it ends the stream and resubscribes",
        (kind) => {
            const m = mount();
            hub.snapshot = { turnPhase: { kind } };
            controllerStatus("done");
            vi.advanceTimersByTime(1499);
            expect(m.dispatched()).toEqual([]);
            vi.advanceTimersByTime(1);
            expect(m.dispatched()).toEqual([
                { type: "StreamUnsubscribe", at: T0 + 1500 },
                { type: "StreamSubscribe", at: T0 + 1500 },
            ]);
            expect(hub.snapshotBlocks.at(-1)).toBe(BLOCK);
        }
    );

    it.each([["Idle"], ["Done"], ["Interrupting"], ["Disconnected"]])(
        "does nothing when the turn is %s by then",
        (kind) => {
            const m = mount();
            hub.snapshot = { turnPhase: { kind } };
            controllerStatus("done");
            vi.advanceTimersByTime(5000);
            expect(m.dispatched()).toEqual([]);
        }
    );

    it("does nothing when the pane has no state by then", () => {
        const m = mount();
        hub.snapshot = null;
        controllerStatus("done");
        vi.advanceTimersByTime(5000);
        expect(m.dispatched()).toEqual([]);
    });

    it("a running status cancels the pending timer, and only that", () => {
        const m = mount();
        hub.snapshot = { turnPhase: { kind: "Streaming" } };
        controllerStatus("done");
        vi.advanceTimersByTime(1000);
        controllerStatus("running");
        vi.advanceTimersByTime(5000);
        expect(m.dispatched()).toEqual([]);
        expect(vi.getTimerCount()).toBe(1);
    });

    it("ignores a status that is neither done nor running", () => {
        const m = mount();
        hub.snapshot = { turnPhase: { kind: "Streaming" } };
        controllerStatus("init");
        expect(vi.getTimerCount()).toBe(1);
        vi.advanceTimersByTime(5000);
        expect(m.dispatched()).toEqual([]);
    });

    it("can be armed again after a new turn cancelled it", () => {
        const m = mount(streaming());
        hub.snapshot = { turnPhase: { kind: "Streaming" } };
        controllerStatus("done");
        m.setPhase(submitting());
        hub.snapshot = { turnPhase: { kind: "Submitting" } };
        vi.advanceTimersByTime(1500);
        expect(m.dispatched()).toEqual([]);
        controllerStatus("done");
        vi.advanceTimersByTime(1500);
        expect(m.dispatched().map((d) => d.type)).toEqual(["StreamUnsubscribe", "StreamSubscribe"]);
    });

    it("arms one timer for a second done, not a second timer", () => {
        const m = mount();
        hub.snapshot = { turnPhase: { kind: "Streaming" } };
        controllerStatus("done");
        vi.advanceTimersByTime(1000);
        controllerStatus("done");
        vi.advanceTimersByTime(500);
        expect(m.dispatched()).toHaveLength(2);
        vi.advanceTimersByTime(5000);
        expect(m.dispatched()).toHaveLength(2);
    });

    it("can be armed again after it fired or was cancelled", () => {
        const m = mount();
        hub.snapshot = { turnPhase: { kind: "Streaming" } };
        controllerStatus("done");
        vi.advanceTimersByTime(1500);
        expect(m.dispatched()).toHaveLength(2);
        controllerStatus("done");
        vi.advanceTimersByTime(1500);
        expect(m.dispatched()).toHaveLength(4);
        controllerStatus("done");
        controllerStatus("running");
        controllerStatus("done");
        vi.advanceTimersByTime(1500);
        expect(m.dispatched()).toHaveLength(6);
    });

    it("a new submitted turn cancels it, a streaming one does not", () => {
        const m = mount(streaming());
        hub.snapshot = { turnPhase: { kind: "Streaming" } };
        controllerStatus("done");
        vi.advanceTimersByTime(500);
        m.setPhase(phase("Streaming", { bufferSize: 1, toolsActive: 0, lastEventMs: T0 + 1 }));
        vi.advanceTimersByTime(1000);
        expect(m.dispatched()).toHaveLength(2);
        controllerStatus("done");
        vi.advanceTimersByTime(500);
        m.setPhase(submitting());
        hub.snapshot = { turnPhase: { kind: "Submitting" } };
        vi.advanceTimersByTime(5000);
        expect(m.dispatched().filter((d) => d.type === "StreamUnsubscribe")).toHaveLength(1);
    });

    it("is dropped, with its subscription, when the pane goes away", () => {
        const m = mount();
        hub.snapshot = { turnPhase: { kind: "Streaming" } };
        controllerStatus("done");
        dispose?.();
        expect(hub.unsubscribed).toContain("controllerstatus");
        vi.advanceTimersByTime(5000);
        expect(m.dispatched()).toEqual([]);
    });
});

describe("the Esc stop-fallback timer", () => {
    it("finishes the turn locally if no session_end arrives within 1.5 s of a stop", () => {
        const m = mount(streaming());
        m.setPhase(interrupting());
        vi.advanceTimersByTime(1499);
        expect(m.dispatched().filter((d) => d.type === "TurnEnd")).toEqual([]);
        vi.advanceTimersByTime(1);
        expect(m.dispatched().filter((d) => d.type === "TurnEnd")).toEqual([{ type: "TurnEnd", stats: null }]);
        expect(m.flushParserPending).toHaveBeenCalledTimes(1);
    });

    it("arms nothing for a turn that is not being stopped", () => {
        const m = mount(streaming());
        vi.advanceTimersByTime(60_000);
        expect(m.dispatched().filter((d) => d.type === "TurnEnd")).toEqual([]);
    });

    it("is cancelled when the turn ends before it fires", () => {
        const m = mount(streaming());
        m.setPhase(interrupting());
        vi.advanceTimersByTime(1000);
        m.setPhase(phase("Done", { outcome: "interrupted", finishedAt: T0 }));
        vi.advanceTimersByTime(5000);
        expect(m.dispatched().filter((d) => d.type === "TurnEnd")).toEqual([]);
    });

    it("is re-armed from the new stop, not carried over from an earlier one", () => {
        const m = mount(streaming());
        m.setPhase(interrupting());
        vi.advanceTimersByTime(1000);
        m.setPhase(streaming());
        m.setPhase(interrupting());
        vi.advanceTimersByTime(1000);
        expect(m.dispatched().filter((d) => d.type === "TurnEnd")).toEqual([]);
        vi.advanceTimersByTime(500);
        expect(m.dispatched().filter((d) => d.type === "TurnEnd")).toHaveLength(1);
    });

    it("is dropped with the pane", () => {
        const m = mount(streaming());
        m.setPhase(interrupting());
        dispose?.();
        vi.advanceTimersByTime(5000);
        expect(m.dispatched().filter((d) => d.type === "TurnEnd")).toEqual([]);
    });

    it("adds the Interrupted row when it finishes the turn", () => {
        const m = mount(streaming());
        m.setPhase(interrupting());
        vi.advanceTimersByTime(1500);
        expect(m.queue.pushNewNode).toHaveBeenCalledTimes(1);
    });
});

describe("the submit-ack timer, further", () => {
    const accepted = () => {
        const h = hub.handlers.get("agent-message-accepted");
        if (!h) throw new Error("no accepted handler");
        h({ data: { message_id: "m" } });
    };

    it("is scoped to this block's accepted events, and stamps the time it fires", () => {
        const m = mount();
        m.setPhase(submitting());
        expect(hub.scopes).toContainEqual({ eventType: "agent-message-accepted", scope: `block:${BLOCK}` });
        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS);
        expect(m.dispatched()).toContainEqual({ type: "SubmitTimeoutElapsed", at: T0 + SUBMIT_TIMEOUT_MS });
    });

    it("an accepted event for this episode stops its timer, and it stops listening", () => {
        const m = mount();
        m.setPhase(submitting());
        accepted();
        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS * 2);
        expect(m.dispatched()).toEqual([]);
        m.setPhase(streaming());
        expect(hub.unsubscribed).toContain("agent-message-accepted");
    });

    it("stops listening when the phase leaves Submitting, and when the pane goes away", () => {
        const m = mount();
        m.setPhase(submitting());
        expect(hub.handlers.has("agent-message-accepted")).toBe(true);
        m.setPhase(streaming());
        expect(hub.handlers.has("agent-message-accepted")).toBe(false);
        m.setPhase(submitting());
        dispose?.();
        expect(hub.handlers.has("agent-message-accepted")).toBe(false);
        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS * 2);
        expect(m.dispatched()).toEqual([]);
    });

    it("arms nothing in any other phase", () => {
        const m = mount();
        for (const p of [
            idle(),
            streaming(),
            interrupting(),
            phase("Done", { outcome: "completed", finishedAt: T0 }),
        ]) {
            m.setPhase(p);
            expect(hub.handlers.has("agent-message-accepted")).toBe(false);
        }
        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS * 2);
        expect(m.dispatched()).toEqual([]);
    });
});

describe("the watchdog", () => {
    it("ticks every five seconds with the time, and stops when the pane goes away", () => {
        const m = mount();
        vi.advanceTimersByTime(4999);
        expect(m.ticks()).toEqual([]);
        vi.advanceTimersByTime(1);
        expect(m.ticks()).toEqual([{ type: "StreamWatchdogTick", nowMs: T0 + 5000 }]);
        vi.advanceTimersByTime(5000);
        expect(m.ticks()).toHaveLength(2);
        dispose?.();
        vi.advanceTimersByTime(60_000);
        expect(m.ticks()).toHaveLength(2);
    });

    it("the catch-up tick when the page turns visible carries the time", () => {
        const m = mount();
        Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
        vi.setSystemTime(T0 + 123);
        document.dispatchEvent(new Event("visibilitychange"));
        expect(m.ticks()).toEqual([{ type: "StreamWatchdogTick", nowMs: T0 + 123 }]);
    });

    it("logs the pane's first seven characters on a visibility change", () => {
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        mount();
        Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
        document.dispatchEvent(new Event("visibilitychange"));
        expect(info).toHaveBeenCalledWith("[wave-turn]", "pane=abcdefg", "visibility: visible→hidden");
        info.mockRestore();
    });
});
