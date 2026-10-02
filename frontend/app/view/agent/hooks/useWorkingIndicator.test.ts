// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { initialState, type AgentPaneState } from "@/app/store/agent-pane-state/types";
import { createRoot } from "solid-js";
import { describe, expect, it } from "vitest";
import type { DocumentNode } from "../types";
import { useWorkingIndicator } from "./useWorkingIndicator";

/** A pane model reduced to what the hook reads: `state` and `document()`. */
function mount(state: Partial<AgentPaneState>, nodes: DocumentNode[] = [], launching = false) {
    return createRoot((dispose) => {
        const w = useWorkingIndicator({
            paneModel: { state: { ...initialState("a"), ...state }, document: () => nodes },
            showingLaunchActivity: () => launching,
        });
        return { ...w, dispose };
    });
}

describe("useWorkingIndicator", () => {
    it("is idle for an idle pane", () => {
        const w = mount({});
        expect([w.paneBusy(), w.workingRowVisible()]).toEqual([false, false]);
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
});
