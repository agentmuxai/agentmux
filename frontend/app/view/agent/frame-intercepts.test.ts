// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { CompactionSummaryTracker } from "./context-delivery";
import { interceptFrame, type FrameInterceptState, type FrameSink } from "./frame-intercepts";
import { ClaudeCodeStreamParser } from "./stream-parser";
import { createTaskWakeDetector } from "./task-wake";

/** A sink that records what each hook was called with, in order. */
function recording() {
    const calls: string[] = [];
    const sink: FrameSink = {
        node: (n) => calls.push(`node:${n.type}`),
        placeReleased: () => calls.push("placeReleased"),
        compactBoundary: (data) => calls.push(`compactBoundary:${data ? "parsed" : "null"}`),
        sessionOutcome: (data) => calls.push(`sessionOutcome:${data?.outcome ?? "null"}`),
        memoryInjected: () => calls.push("memoryInjected"),
    };
    return { calls, sink };
}

function state(): FrameInterceptState & { flushes: number; cleared: number } {
    const parser = new ClaudeCodeStreamParser({ isReplay: true });
    const st = { parser, compactionSummaries: new CompactionSummaryTracker(), detectTaskWake: createTaskWakeDetector(), flushes: 0, cleared: 0 };
    const flush = parser.flushPending.bind(parser);
    parser.flushPending = () => {
        st.flushes++;
        return flush();
    };
    const clear = parser.clearHiddenReinjectionState.bind(parser);
    parser.clearHiddenReinjectionState = () => {
        st.cleared++;
        clear();
    };
    return st;
}

const boundary = {
    type: "system",
    subtype: "compact_boundary",
    timestamp: "2026-10-09T12:00:00Z",
    compact_metadata: { trigger: "auto", pre_tokens: 150_000, post_tokens: 9_000, duration_ms: 30_000 },
};
const summary = {
    type: "user",
    isSynthetic: true,
    message: { role: "user", content: "This session is being continued from a previous conversation that ran out of context." },
};

describe("interceptFrame", () => {
    it("closes the open block before a compaction boundary, even one that doesn't parse", () => {
        for (const frame of [boundary, { type: "system", subtype: "compact_boundary" }]) {
            const st = state();
            const { calls, sink } = recording();
            expect(interceptFrame(frame, st, sink, 1_000)).toBe(true);
            expect(st.flushes).toBe(1);
            expect(calls).toEqual(["placeReleased", `compactBoundary:${frame === boundary ? "parsed" : "null"}`]);
        }
    });

    it("pairs a boundary with the summary after it: the summary is a node, not a message", () => {
        const st = state();
        const { calls, sink } = recording();
        interceptFrame(boundary, st, sink, 1_000);
        expect(interceptFrame(summary, st, sink, 2_000)).toBe(true);
        expect(calls.slice(2)).toEqual(["placeReleased", "node:context_delivery"]);
    });

    it("ends a hidden memory re-injection's suppression at any session boundary, live and replayed alike", () => {
        for (const outcome of ["resumed", "fresh", "garbage"]) {
            const st = state();
            const { calls, sink } = recording();
            const frame = { type: "system", subtype: "agentmux_session_outcome", outcome, attempted_sid: "s1" };
            expect(interceptFrame(frame, st, sink, 1_000)).toBe(true);
            expect(st.cleared).toBe(1);
            expect(calls).toEqual(["placeReleased", `sessionOutcome:${outcome === "garbage" ? "null" : outcome}`]);
        }
    });

    it("leaves the memory notice's block-closing to the path, and lets ordinary frames through", () => {
        const st = state();
        const { calls, sink } = recording();
        expect(interceptFrame({ type: "system", subtype: "agentmux_memory_injected" }, st, sink, 1_000)).toBe(true);
        expect(st.flushes).toBe(0);
        expect(calls).toEqual(["memoryInjected"]);
        expect(interceptFrame({ type: "assistant", message: { content: [] } }, st, sink, 1_000)).toBe(false);
        expect(interceptFrame({ type: "system", subtype: "init" }, st, sink, 1_000)).toBe(false);
    });

    it("notes a task wake and still lets the frame through", () => {
        const st = state();
        const { calls, sink } = recording();
        interceptFrame({ type: "system", subtype: "task_notification", task_id: "t1", summary: "done" }, st, sink, 1_000);
        expect(interceptFrame({ type: "system", subtype: "init" }, st, sink, 2_000)).toBe(false);
        expect(calls).toEqual(["node:ambient_narration"]);
    });
});
