// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { initialState, type AgentPaneState } from "@/app/store/agent-pane-state/types";
import { createRoot, createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import type { DocumentNode, ToolNode } from "../types";
import { useWorkingIndicator } from "./useWorkingIndicator";

const bash = (status: ToolNode["status"], startedAt: number): ToolNode => ({
    type: "tool",
    id: `t-${startedAt}`,
    tool: "Bash",
    status,
    params: { command: "cargo test" },
    collapsed: false,
    summary: "",
    timestamp: startedAt,
});

/** A pane model reduced to what the hook reads: `state` and `document()`. */
function mount(state: Partial<AgentPaneState>, nodes: DocumentNode[] = [], launching = false) {
    return createRoot((dispose) => {
        const [tick, setTick] = createSignal(0);
        const w = useWorkingIndicator({
            paneModel: { state: { ...initialState("a"), ...state }, document: () => nodes },
            showingLaunchActivity: () => launching,
            promotionTick: tick,
        });
        return { ...w, bump: () => setTick((n) => n + 1), dispose };
    });
}

describe("useWorkingIndicator", () => {
    it("is idle for an idle pane", () => {
        const w = mount({});
        expect([w.paneBusy(), w.workingRowVisible(), w.hasPromotedTool()]).toEqual([false, false, false]);
        w.dispose();
    });

    it("is busy while launching", () => {
        const w = mount({}, [], true);
        expect([w.paneBusy(), w.workingRowVisible()]).toEqual([true, true]);
        w.dispose();
    });

    it("keeps the working row up after a turn that has stats to show", () => {
        const w = mount({ sessionStats: { input_tokens: 1, output_tokens: 2 } });
        expect([w.paneBusy(), w.workingRowVisible()]).toEqual([false, true]);
        w.dispose();
    });

    it("reports a promoted tool once a running call is past the threshold", () => {
        const w = mount({}, [bash("running", 0)]);
        // Date.now() is far past a timestamp of 0, so the call is promoted.
        expect(w.hasPromotedTool()).toBe(true);
        w.dispose();
    });

    it("doesn't count a finished call as promoted", () => {
        const w = mount({}, [bash("success", 0)]);
        expect(w.hasPromotedTool()).toBe(false);
        w.dispose();
    });
});
