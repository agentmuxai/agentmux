// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";

import {
    __resetAllSlots,
    type AgentBlockResult,
    dispatch,
    registerPane,
    setEventSink,
    snapshot,
    unregisterPane,
    type WorkflowRunStatus,
} from "./workflow-run-state-store";
import type { WorkflowRunEvent } from "./workflow-run-state/types";

interface Projections {
    closed: boolean[];
    runId: string[];
    workflowId: string[];
    status: WorkflowRunStatus[];
    blockResults: Record<string, AgentBlockResult>[];
    output: string[];
    error: string[];
}

function mkProj(): {
    calls: Projections;
    proj: Parameters<typeof registerPane>[1];
} {
    const calls: Projections = {
        closed: [],
        runId: [],
        workflowId: [],
        status: [],
        blockResults: [],
        output: [],
        error: [],
    };
    return {
        calls,
        proj: {
            closed: (v) => calls.closed.push(v),
            runId: (v) => calls.runId.push(v),
            workflowId: (v) => calls.workflowId.push(v),
            status: (v) => calls.status.push(v),
            blockResults: (v) => calls.blockResults.push(v),
            output: (v) => calls.output.push(v),
            error: (v) => calls.error.push(v),
        },
    };
}

describe("workflow-run-state-store (slice #10)", () => {
    afterEach(() => {
        __resetAllSlots();
        setEventSink(() => {});
    });

    it("dispatch on unregistered blockId throws (no silent drops)", () => {
        expect(() =>
            dispatch("nope", { type: "RunStarted", runId: "r1", workflowId: "w1" }),
        ).toThrowError(/unregistered pane/);
    });

    it("registers a pane and projects only changed cells", () => {
        const { proj, calls } = mkProj();
        registerPane("blk-1", proj);
        dispatch("blk-1", { type: "RunStarted", runId: "r1", workflowId: "w1" });
        expect(calls.runId).toEqual(["r1"]);
        expect(calls.workflowId).toEqual(["w1"]);
        expect(calls.status).toEqual(["running"]);
        expect(calls.closed).toEqual([]);
        // RunStarted always allocates a fresh empty blockResults map
        // even if the prior was empty — the projector fires once on
        // reference change. The view treats this as "reset for new run."
        expect(calls.blockResults).toEqual([{}]);
    });

    it("BlockDone projects the freshly-allocated blockResults map", () => {
        const { proj, calls } = mkProj();
        registerPane("blk-1", proj);
        dispatch("blk-1", { type: "RunStarted", runId: "r1", workflowId: "w1" });
        dispatch("blk-1", {
            type: "BlockDone",
            blockId: "agent-1",
            output: { response: "hello", cost_usd: 0.001 },
        });
        // RunStarted clears blockResults to a NEW empty {} (initialState
        // had {} too — different reference triggers projection on the
        // start). BlockDone allocates ANOTHER new map. So we see 2 calls.
        expect(calls.blockResults.length).toBeGreaterThanOrEqual(1);
        const last = calls.blockResults[calls.blockResults.length - 1];
        expect(last["agent-1"]).toEqual({ response: "hello", costUsd: 0.001 });
    });

    it("RunDone projects status without clobbering blockResults", () => {
        const { proj, calls } = mkProj();
        registerPane("blk-1", proj);
        dispatch("blk-1", { type: "RunStarted", runId: "r1", workflowId: "w1" });
        dispatch("blk-1", {
            type: "BlockDone",
            blockId: "agent-1",
            output: { response: "hi" },
        });
        dispatch("blk-1", { type: "RunDone", output: "final" });
        expect(calls.status[calls.status.length - 1]).toBe("done");
        expect(snapshot("blk-1")?.blockResults["agent-1"].response).toBe("hi");
        expect(snapshot("blk-1")?.output).toBe("final");
    });

    it("BackfilledFromRow overwrites blockResults and flips status", () => {
        const { proj } = mkProj();
        registerPane("blk-1", proj);
        dispatch("blk-1", {
            type: "BackfilledFromRow",
            runId: "r1",
            workflowId: "w1",
            status: "done",
            output: "x",
            error: "",
            blocks: [
                { blockId: "a", status: "done", output: { response: "ok" } },
                { blockId: "b", status: "error", error: "no" },
            ],
        });
        const snap = snapshot("blk-1");
        expect(snap?.status).toBe("done");
        expect(snap?.blockResults["a"].response).toBe("ok");
        expect(snap?.blockResults["b"].error).toBe("no");
    });

    it("emits events through the configured sink", () => {
        const events: { blockId: string; ev: WorkflowRunEvent }[] = [];
        setEventSink((blockId, ev) => events.push({ blockId, ev }));
        const { proj } = mkProj();
        registerPane("blk-1", proj);
        dispatch("blk-1", { type: "RunStarted", runId: "r1", workflowId: "w1" });
        expect(events).toHaveLength(1);
        expect(events[0].ev.type).toBe("run-started");
    });

    it("Disposed gate: post-close commands don't reach projectors", () => {
        const { proj, calls } = mkProj();
        registerPane("blk-1", proj);
        dispatch("blk-1", { type: "Disposed" });
        const closedCallsBefore = calls.closed.length;
        const statusCallsBefore = calls.status.length;
        dispatch("blk-1", { type: "RunStarted", runId: "r1", workflowId: "w1" });
        expect(calls.closed.length).toBe(closedCallsBefore);
        expect(calls.status.length).toBe(statusCallsBefore);
        expect(snapshot("blk-1")?.runId).toBe("");
    });

    it("unregisterPane removes the slot", () => {
        const { proj } = mkProj();
        registerPane("blk-1", proj);
        unregisterPane("blk-1");
        expect(snapshot("blk-1")).toBeNull();
        expect(() =>
            dispatch("blk-1", { type: "RunStarted", runId: "r1", workflowId: "w1" }),
        ).toThrowError(/unregistered pane/);
    });
});
