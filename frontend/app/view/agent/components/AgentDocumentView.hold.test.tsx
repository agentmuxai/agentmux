// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A tool that finishes while the pane is shown is held open; one that finishes
 * while the pane is hidden isn't: nobody watched it finish, and an unheld row's
 * result can leave memory like any collapsed row's (#4847's bound on hidden
 * panes). REPORT_AGENT_PANE_HELD_TOOLS_COLLAPSE_ON_RETURN_RCA_2026_10_17.md.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { makeTestHostApi } from "@/app/host/test-host";
import { AgentDormancyProvider } from "../agent-dormancy";
import { createAgentAtoms } from "../state";
import type { DocumentNode, ToolNode } from "../types";
import { AgentDocumentView } from "./AgentDocumentView";

class FakeResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
}

const tool = (status: ToolNode["status"]): ToolNode => ({
    type: "tool",
    id: "tc-1",
    tool: "Bash",
    params: { command: "ls" },
    status,
    collapsed: true,
    summary: "Bash ls",
});

function setup(dormant: () => boolean) {
    const [nodes, setNodes] = createSignal<DocumentNode[]>([tool("running")]);
    const atoms = createAgentAtoms();
    render(() => (
        <AgentDormancyProvider dormant={dormant}>
            <AgentDocumentView documentNodes={nodes} documentStateAtom={atoms.documentStateAtom} />
        </AgentDormancyProvider>
    ));
    return { finish: () => setNodes([tool("success")]), held: () => atoms.documentStateAtom[0]().heldOpenNodes };
}

describe("AgentDocumentView — holding a tool that finishes", () => {
    beforeEach(() => {
        window.api = makeTestHostApi({});
        vi.stubGlobal("ResizeObserver", FakeResizeObserver);
        vi.stubGlobal("requestAnimationFrame", () => 0);
        vi.stubGlobal("cancelAnimationFrame", () => {});
    });
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it("holds a tool that finishes while the pane is shown", () => {
        const pane = setup(() => false);
        pane.finish();
        expect(pane.held().has("tc-1")).toBe(true);
    });

    it("doesn't hold a tool that finishes while the pane is hidden", () => {
        const pane = setup(() => true);
        pane.finish();
        expect(pane.held().has("tc-1")).toBe(false);
    });
});
