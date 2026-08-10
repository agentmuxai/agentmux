// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Coverage for `flushNow()` — the synchronous flush added for the
 * session_end path (turn-tail flush race: the turn's own trailing document
 * nodes must land BEFORE TurnEnd settles the phase, or the reducer's
 * StreamFlushObserved re-promotion reads them as a new round and the pane
 * sticks on a false "Working…" after the turn genuinely ended). No test
 * file existed for this module before.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RpcApi } from "@/app/store/rpc-api";
import { createStreamFlushQueue } from "./stream-flush-queue";
import type { DocumentNode, ToolNode } from "./types";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: vi.fn().mockResolvedValue(undefined) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

type Dispatched = { kind: "doc" | "pane"; command: any };

function makeModel() {
    const dispatched: Dispatched[] = [];
    const model = {
        blockId: "block-1",
        dispatchDoc: (command: any) => { dispatched.push({ kind: "doc", command }); },
        dispatchPane: (command: any) => { dispatched.push({ kind: "pane", command }); },
    };
    return { model: model as any, dispatched };
}

const textNode = (id: string): DocumentNode => ({ type: "markdown", id, content: "hi" }) as DocumentNode;

describe("StreamFlushQueue.flushNow", () => {
    let rafQueue: FrameRequestCallback[];

    beforeEach(() => {
        rafQueue = [];
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            rafQueue.push(cb);
            return rafQueue.length;
        });
        vi.stubGlobal("cancelAnimationFrame", (id: number) => {
            rafQueue[id - 1] = () => {};
        });
    });

    afterEach(() => vi.unstubAllGlobals());

    function flushRaf() {
        const pending = rafQueue;
        rafQueue = [];
        for (const cb of pending) cb(0);
    }

    it("flushes pending nodes synchronously, without waiting for the RAF", () => {
        const { model, dispatched } = makeModel();
        const q = createStreamFlushQueue(model);

        q.pushNewNode(textNode("n1"));
        q.scheduleFlush();
        expect(dispatched).toHaveLength(0); // still queued behind the RAF

        q.flushNow();

        const kinds = dispatched.map((d) => d.command.type);
        expect(kinds).toContain("StreamFlush");
        expect(kinds).toContain("StreamFlushObserved");
        const flush = dispatched.find((d) => d.command.type === "StreamFlush")!;
        expect(flush.command.newNodes.map((n: DocumentNode) => n.id)).toEqual(["n1"]);
    });

    it("the armed RAF does not double-dispatch after flushNow", () => {
        const { model, dispatched } = makeModel();
        const q = createStreamFlushQueue(model);

        q.pushNewNode(textNode("n1"));
        q.scheduleFlush();
        q.flushNow();
        const countAfterFlushNow = dispatched.length;

        flushRaf();

        // No second StreamFlush/StreamFlushObserved — a duplicate (even an
        // empty one) landing after TurnEnd would re-promote Done → Streaming,
        // the exact bug flushNow exists to prevent.
        expect(dispatched).toHaveLength(countAfterFlushNow);
    });

    it("is a no-op with nothing pending (never dispatches an empty StreamFlushObserved)", () => {
        const { model, dispatched } = makeModel();
        const q = createStreamFlushQueue(model);

        q.flushNow();

        expect(dispatched).toHaveLength(0);
    });

    it("a later scheduleFlush with new content still flushes normally (genuine next round unaffected)", () => {
        const { model, dispatched } = makeModel();
        const q = createStreamFlushQueue(model);

        q.pushNewNode(textNode("n1"));
        q.flushNow();
        const afterFirst = dispatched.length;

        q.pushNewNode(textNode("n2"));
        q.scheduleFlush();
        flushRaf();

        expect(dispatched.length).toBeGreaterThan(afterFirst);
        const flushes = dispatched.filter((d) => d.command.type === "StreamFlush");
        expect(flushes[1].command.newNodes.map((n: DocumentNode) => n.id)).toEqual(["n2"]);
    });

    it("issue #2518: pushDockNodeStatus forwards run_in_background so muxspect dock can tell a background launch's 'success' apart from an ordinary finished call's", () => {
        const { model } = makeModel();
        const q = createStreamFlushQueue(model);

        const bgBash: ToolNode = {
            type: "tool",
            id: "toolu_bg",
            tool: "Bash",
            status: "success",
            params: { command: "task dev", run_in_background: true },
            collapsed: false,
            summary: "",
            timestamp: 1000,
        };
        q.pushNewNode(bgBash);

        const calls = vi.mocked(RpcApi.DockNodeStatusCommand).mock.calls;
        const call = calls.find(([, data]) => data.node_id === "toolu_bg");
        expect(call?.[1].run_in_background).toBe(true);

        // An ordinary Bash call (no run_in_background) sends undefined, not
        // false — the server-side field stays absent (skip_serializing_if)
        // rather than misleadingly asserting "definitely not background."
        const fgBash: ToolNode = { ...bgBash, id: "toolu_fg", params: { command: "ls" } };
        q.pushNewNode(fgBash);
        const fgCall = calls.find(([, data]) => data.node_id === "toolu_fg");
        expect(fgCall?.[1].run_in_background).toBeUndefined();
    });
});
