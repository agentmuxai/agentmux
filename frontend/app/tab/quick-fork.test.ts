// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
// vi.mock calls below are hoisted above this import by vitest — see
// open-history-tab.test.ts's own note on why this ordering is safe despite
// not using a dynamic import.
import { quickForkTabToNewTab, resolveActiveAgentForTab } from "./quick-fork";

const focusedNode = vi.fn();
const getLayoutModelForTabById = vi.fn();
vi.mock("@/layout/index", () => ({
    getLayoutModelForTabById: (...args: unknown[]) => getLayoutModelForTabById(...args),
}));

const getObjectValue = vi.fn();
const getBlockComponentModel = vi.fn();
vi.mock("@/app/store/global", () => ({
    getBlockComponentModel: (...args: unknown[]) => getBlockComponentModel(...args),
    WOS: {
        getObjectValue: (...args: unknown[]) => getObjectValue(...args),
        makeORef: (kind: string, id: string) => `${kind}:${id}`,
    },
}));

const forkAgentDefinitionCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ForkAgentDefinitionCommand: (...args: unknown[]) => forkAgentDefinitionCommand(...args) },
}));

const rpcCall = vi.fn();
vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: { rpcCall: (...args: unknown[]) => rpcCall(...args) },
}));

const createTab = vi.fn();
vi.mock("@/app/store/services", () => ({
    WorkspaceService: { CreateTab: (...args: unknown[]) => createTab(...args) },
}));

const workspace = vi.fn();
vi.mock("@/app/store/window-identity", () => ({
    workspace: () => workspace(),
}));

vi.mock("@/util/logger", () => ({
    Logger: { warn: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

describe("resolveActiveAgentForTab", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        getLayoutModelForTabById.mockReturnValue({ focusedNode });
    });

    it("returns null when the tab has no layout model", () => {
        getLayoutModelForTabById.mockReturnValue(undefined);
        expect(resolveActiveAgentForTab("tab-1")).toBeNull();
    });

    it("returns null when nothing is focused in the tab", () => {
        focusedNode.mockReturnValue(null);
        expect(resolveActiveAgentForTab("tab-1")).toBeNull();
    });

    it("returns null when the focused node has no block id", () => {
        focusedNode.mockReturnValue({ data: {} });
        expect(resolveActiveAgentForTab("tab-1")).toBeNull();
    });

    it("returns null for a non-agent-view block", () => {
        focusedNode.mockReturnValue({ data: { blockId: "block-1" } });
        getObjectValue.mockReturnValue({ meta: { view: "term" } });
        expect(resolveActiveAgentForTab("tab-1")).toBeNull();
    });

    it("returns null for an agent-view block with no agentId yet (blank picker tab)", () => {
        focusedNode.mockReturnValue({ data: { blockId: "block-1" } });
        getObjectValue.mockReturnValue({ meta: { view: "agent" } });
        expect(resolveActiveAgentForTab("tab-1")).toBeNull();
    });

    it("prefers activeBlockId over blockId (in-pane fork-bar block-stack case)", () => {
        focusedNode.mockReturnValue({ data: { blockId: "stack-root", activeBlockId: "stack-active" } });
        getObjectValue.mockImplementation((oref: string) =>
            oref === "block:stack-active"
                ? { meta: { view: "agent", agentId: "def-1", "agent:sessionid": "sid-1" } }
                : { meta: { view: "agent", agentId: "wrong-def" } }
        );
        expect(resolveActiveAgentForTab("tab-1")).toEqual({
            blockId: "stack-active",
            definitionId: "def-1",
            sessionId: "sid-1",
        });
    });

    it("resolves a live agent block, defaulting sessionId to empty when not yet set", () => {
        focusedNode.mockReturnValue({ data: { blockId: "block-1" } });
        getObjectValue.mockReturnValue({ meta: { view: "agent", agentId: "def-1" } });
        expect(resolveActiveAgentForTab("tab-1")).toEqual({
            blockId: "block-1",
            definitionId: "def-1",
            sessionId: "",
        });
    });
});

describe("quickForkTabToNewTab", () => {
    const launchAgentDefinition = vi.fn();

    beforeEach(() => {
        vi.clearAllMocks();
        getLayoutModelForTabById.mockReturnValue({ focusedNode });
        focusedNode.mockReturnValue({ data: { blockId: "source-block" } });
        getObjectValue.mockReturnValue({ meta: { view: "agent", agentId: "source-def", "agent:sessionid": "sid-parent" } });
        getBlockComponentModel.mockReturnValue({ viewModel: { launchAgentDefinition } });
        workspace.mockReturnValue({ oid: "ws-1" });
        forkAgentDefinitionCommand.mockResolvedValue({ id: "forked-def", name: "X #2", agent_type: "host" });
        createTab.mockResolvedValue("new-tab-1");
        rpcCall.mockResolvedValue({ block_id: "new-block-1" });
        launchAgentDefinition.mockResolvedValue(true);
    });

    afterEach(() => {
        vi.restoreAllMocks();
    });

    it("returns null when the source tab has no active agent", async () => {
        getObjectValue.mockReturnValue(undefined);
        expect(await quickForkTabToNewTab("tab-1")).toBeNull();
        expect(forkAgentDefinitionCommand).not.toHaveBeenCalled();
    });

    it("returns null when the source block has no live view model", async () => {
        getBlockComponentModel.mockReturnValue(undefined);
        expect(await quickForkTabToNewTab("tab-1")).toBeNull();
        expect(forkAgentDefinitionCommand).not.toHaveBeenCalled();
    });

    it("returns null when there's no current workspace", async () => {
        workspace.mockReturnValue(undefined);
        expect(await quickForkTabToNewTab("tab-1")).toBeNull();
        expect(forkAgentDefinitionCommand).not.toHaveBeenCalled();
    });

    it("forks the definition, creates a new tab, opens a block into it via an explicit tab_id, and launches with history carryover + unbound identity", async () => {
        const newTabId = await quickForkTabToNewTab("tab-1");

        expect(forkAgentDefinitionCommand).toHaveBeenCalledWith(
            expect.anything(),
            { source_id: "source-def", branch_label: "" }
        );
        expect(createTab).toHaveBeenCalledWith("ws-1", "X #2", true, false);
        // pane.open must place the block directly into the NEW tab via an
        // explicit tab_id (open_pane's "explicit tab_id wins" path) — not
        // skip_placement, which is the different in-pane block-stack dance.
        expect(rpcCall).toHaveBeenCalledWith(
            "pane.open",
            { view: "agent", tab_id: "new-tab-1", meta: { view: "agent" } },
            {}
        );
        expect(launchAgentDefinition).toHaveBeenCalledTimes(1);
        const [forkedDef, overrides, targetBlockId, targetTabId] = launchAgentDefinition.mock.calls[0];
        expect(forkedDef).toEqual({ id: "forked-def", name: "X #2", agent_type: "host" });
        // History carryover (Phase 1, PR #2725's continueSessionId/forkSession).
        expect(overrides.continueSessionId).toBe("sid-parent");
        expect(overrides.forkSession).toBe(true);
        // Spec §5: unbound identity by default.
        expect(overrides.accountId).toBe("");
        expect(overrides.memoryId).toBe("");
        // Explicit cross-tab targeting — never ambient active-tab/block state.
        expect(targetBlockId).toBe("new-block-1");
        expect(targetTabId).toBe("new-tab-1");
        expect(newTabId).toBe("new-tab-1");
    });

    it("still returns the new tab id even if launchAgentDefinition itself reports failure (best-effort logging, not a thrown error)", async () => {
        launchAgentDefinition.mockResolvedValue(false);
        expect(await quickForkTabToNewTab("tab-1")).toBe("new-tab-1");
    });

    it("returns null if ForkAgentDefinitionCommand rejects", async () => {
        forkAgentDefinitionCommand.mockRejectedValue(new Error("boom"));
        expect(await quickForkTabToNewTab("tab-1")).toBeNull();
    });
});
