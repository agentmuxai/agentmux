// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// A switch that changes must do what it says: the holds and folds of the rows it governs go.

import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({ pane: undefined as unknown, setBlockMeta: undefined as unknown }));
vi.mock("@/app/store/block-atom-cache", () => ({
    getBlockMetaKeyAtom: () => () => hub.pane,
    getSettingsKeyAtom: () => () => undefined,
}));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: (...args: unknown[]) => (hub.setBlockMeta as (...a: unknown[]) => void)(...args) }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { createAgentViewControls } from "./agent-view-controls";
import type { DocumentNode, DocumentState } from "./types";

const tool = { type: "tool", id: "t1", tool: "Read", params: {}, status: "success", collapsed: false } as unknown as DocumentNode;
const thought = { type: "markdown", id: "m1", metadata: { thinking: true } } as unknown as DocumentNode;

function state(over: Partial<DocumentState>): DocumentState {
    return { collapsedNodes: new Set(), heldOpenNodes: new Set(), pinnedNodes: new Set(), ...over } as DocumentState;
}

function run(prev: DocumentState, act: (c: ReturnType<typeof createAgentViewControls>) => void): DocumentState {
    let next = prev;
    hub.setBlockMeta = vi.fn();
    createRoot((dispose) => {
        const c = createAgentViewControls({
            blockId: "b1",
            nodes: () => [tool, thought],
            setDocumentState: (update) => (next = update(next)),
        });
        act(c);
        dispose();
    });
    return next;
}

describe("changing the view", () => {
    it("drops the hold on a tool that finished on screen when Tool calls goes to Hidden", () => {
        const next = run(state({ heldOpenNodes: new Set(["t1", "other"]) }), (c) => c.setPreset("replies"));
        expect([...next.heldOpenNodes]).toEqual(["other"]);
    });

    it("drops a hand-made fold of the rows a switch governs, and keeps a pin", () => {
        const next = run(state({ collapsedNodes: new Set(["t1", "m1"]), pinnedNodes: new Set(["t1"]) }), (c) => c.setSwitch("tools", "hidden"));
        expect([...next.collapsedNodes]).toEqual(["m1"]);
        expect([...next.pinnedNodes]).toEqual(["t1"]);
    });

    it("leaves the state alone when no switch that governs a row changed", () => {
        const prev = state({ heldOpenNodes: new Set(["t1"]) });
        expect(run(prev, (c) => c.setSwitch("notes", "hidden"))).toBe(prev);
    });
});
