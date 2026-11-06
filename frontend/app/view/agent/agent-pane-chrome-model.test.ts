// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The agent pane's tab strip holds this pane's own stack members and nothing else: a block that lives in
// another pane is never drawn as a tab here, because that made two panes look like one group, and
// clicking the "tab" acted on the other pane (#27069, #23913).

import { createRoot } from "solid-js";
import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    setActive: vi.fn(),
    close: vi.fn(),
    leaf: { id: "leaf-1", data: { blockId: "b-a", blockStack: ["b-a", "b-b"], activeBlockId: "b-a" } } as {
        id: string;
        data: { blockId: string; blockStack?: string[]; activeBlockId?: string };
    },
}));

vi.mock("@/app/store/global", () => ({
    MOS: { getMuxObjectAtom: () => () => ({ meta: { agentId: "def-a" } }), makeORef: (t: string, id: string) => `${t}:${id}` },
}));
vi.mock("@/layout/index", () => ({ setActiveBlockInStack: h.setActive }));
vi.mock("@/layout/lib/layoutNode", () => ({ findNode: () => h.leaf }));
vi.mock("./close-agent-tab", () => ({ closeAgentTab: h.close }));

import { buildAgentPaneChromeModel } from "./agent-pane-chrome-model";

function build() {
    const nodeModel = {
        nodeId: "leaf-1",
        activeBlockId: () => "b-a",
        layoutModel: { treeState: { rootNode: {} } },
    } as unknown as Parameters<typeof buildAgentPaneChromeModel>[1];
    return createRoot(() => buildAgentPaneChromeModel("b-a", nodeModel));
}

beforeEach(() => {
    h.setActive.mockReset();
    h.close.mockReset();
});

describe("the agent pane's chrome model", () => {
    it("offers no tabs beyond the pane's own stack", () => {
        expect(build().extraTabs).toBeUndefined();
    });

    it("switches to a member of its own stack in place", () => {
        expect(build().onActivate?.("b-b")).toBe(true);
        expect(h.setActive).toHaveBeenCalledWith(expect.anything(), "leaf-1", "b-b");
    });

    it("does nothing for a block this pane's stack does not hold", () => {
        build().onActivate?.("b-elsewhere");
        expect(h.setActive).not.toHaveBeenCalled();
    });
});
