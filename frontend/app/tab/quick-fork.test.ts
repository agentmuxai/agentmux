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
const listAgentIdentitiesCommand = vi.fn();
const setMetaCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ForkAgentDefinitionCommand: (...args: unknown[]) => forkAgentDefinitionCommand(...args),
        ListAgentIdentitiesCommand: (...args: unknown[]) => listAgentIdentitiesCommand(...args),
        SetMetaCommand: (...args: unknown[]) => setMetaCommand(...args),
    },
}));

const resolveEffectiveLaunchProvider = vi.fn();
vi.mock("@/app/view/agent/agent-launch-env", () => ({
    resolveEffectiveLaunchProvider: (...args: unknown[]) => resolveEffectiveLaunchProvider(...args),
}));

const resolveProviderAlias = vi.fn();
vi.mock("@/app/view/agent/providers", () => ({
    PROVIDERS: {
        claude: { id: "claude" },
        codex: { id: "codex" },
    },
    resolveProviderAlias: (...args: unknown[]) => resolveProviderAlias(...args),
}));

vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: {},
}));

const createTab = vi.fn();
vi.mock("@/app/store/services", () => ({
    WorkspaceService: { CreateTab: (...args: unknown[]) => createTab(...args) },
}));

const workspace = vi.fn();
vi.mock("@/app/store/window-identity", () => ({
    workspace: () => workspace(),
}));

// The proven create+place path (Codex's review of PR #2727): a raw
// pane.open with an explicit tab_id looks correct but is confirmed NOT
// equivalent for a brand-new tab (see quick-fork.ts's own doc comment) —
// waitForLayoutModel + createBlockOnModel is the only path that actually
// renders.
const waitForLayoutModel = vi.fn();
const createBlockOnModel = vi.fn();
const resolveBlockDef = vi.fn();
vi.mock("./tab-presets", () => ({
    waitForLayoutModel: (...args: unknown[]) => waitForLayoutModel(...args),
    createBlockOnModel: (...args: unknown[]) => createBlockOnModel(...args),
    resolveBlockDef: (...args: unknown[]) => resolveBlockDef(...args),
}));

vi.mock("@/util/logger", () => ({
    Logger: { warn: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

// Real string constants, mocked directly (not the real module) to avoid
// pulling in open-history-tab.ts's own dependency graph (WOS, TabRpcClient,
// etc.) into this unit test.
vi.mock("@/app/view/agent/open-history-tab", () => ({
    HISTORY_TAB_FOR_META_KEY: "agent:historyTabFor",
    HISTORY_SOURCE_BLOCK_ID_META_KEY: "agent:historySourceBlockId",
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

    // reagent's review of PR #2727 — an Agent History reader shares the
    // live agent's `agentId` but is never itself launched (no
    // agent:sessionid), so naively treating it as "the active agent"
    // would silently fork with an empty session and no history, no warning.
    describe("Agent History reader exclusion (reagent's review of PR #2727)", () => {
        it("falls back to the reader's recorded source block, not the reader itself", () => {
            focusedNode.mockReturnValue({ data: { blockId: "history-block" } });
            getObjectValue.mockImplementation((oref: string) => {
                if (oref === "block:history-block") {
                    return {
                        meta: {
                            view: "agent",
                            agentId: "def-1",
                            "agent:historyTabFor": "def-1",
                            "agent:historySourceBlockId": "live-block",
                        },
                    };
                }
                if (oref === "block:live-block") {
                    return { meta: { view: "agent", agentId: "def-1", "agent:sessionid": "sid-live" } };
                }
                return undefined;
            });
            expect(resolveActiveAgentForTab("tab-1")).toEqual({
                blockId: "live-block",
                definitionId: "def-1",
                sessionId: "sid-live",
            });
        });

        it("returns null when a history reader has no recorded source block", () => {
            focusedNode.mockReturnValue({ data: { blockId: "history-block" } });
            getObjectValue.mockReturnValue({
                meta: { view: "agent", agentId: "def-1", "agent:historyTabFor": "def-1" },
            });
            expect(resolveActiveAgentForTab("tab-1")).toBeNull();
        });

        it("returns null when a history reader's recorded source block no longer resolves to a live agent", () => {
            focusedNode.mockReturnValue({ data: { blockId: "history-block" } });
            getObjectValue.mockImplementation((oref: string) => {
                if (oref === "block:history-block") {
                    return {
                        meta: {
                            view: "agent",
                            agentId: "def-1",
                            "agent:historyTabFor": "def-1",
                            "agent:historySourceBlockId": "gone-block",
                        },
                    };
                }
                return undefined; // source block deleted/gone
            });
            expect(resolveActiveAgentForTab("tab-1")).toBeNull();
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
        waitForLayoutModel.mockResolvedValue({ treeReducer: vi.fn() });
        resolveBlockDef.mockReturnValue({ meta: { view: "agent" } });
        createBlockOnModel.mockResolvedValue("new-block-1");
        launchAgentDefinition.mockResolvedValue(true);
        resolveEffectiveLaunchProvider.mockResolvedValue("claude");
        resolveProviderAlias.mockImplementation((id: string) => id);
        listAgentIdentitiesCommand.mockResolvedValue([]);
        setMetaCommand.mockResolvedValue(undefined);
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

    it("forks the definition, creates a new tab, places a block via the proven layout-model path, and launches with history carryover + unbound identity", async () => {
        const newTabId = await quickForkTabToNewTab("tab-1");

        expect(forkAgentDefinitionCommand).toHaveBeenCalledWith(
            expect.anything(),
            { source_id: "source-def", branch_label: "" }
        );
        expect(createTab).toHaveBeenCalledWith("ws-1", "X #2", true, false);
        // Must go through waitForLayoutModel + createBlockOnModel, NOT a raw
        // pane.open with an explicit tab_id — confirmed (Codex's review of
        // PR #2727, quick-fork.ts's own doc comment) that pane.open succeeds
        // server-side but never renders for a brand-new tab.
        expect(waitForLayoutModel).toHaveBeenCalledWith("new-tab-1");
        expect(resolveBlockDef).toHaveBeenCalledWith("defwidget@agent");
        expect(createBlockOnModel).toHaveBeenCalledWith(
            "new-tab-1",
            { treeReducer: expect.any(Function) },
            { meta: { view: "agent" } },
            null,
            null
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

    it("returns null (does not fall back to pane.open) if the new tab's layout model never becomes ready", async () => {
        waitForLayoutModel.mockResolvedValue(null);
        expect(await quickForkTabToNewTab("tab-1")).toBeNull();
        expect(createBlockOnModel).not.toHaveBeenCalled();
        expect(launchAgentDefinition).not.toHaveBeenCalled();
    });

    it("returns null if the agent widget's blockdef can't be resolved", async () => {
        resolveBlockDef.mockReturnValue(null);
        expect(await quickForkTabToNewTab("tab-1")).toBeNull();
        expect(createBlockOnModel).not.toHaveBeenCalled();
        expect(launchAgentDefinition).not.toHaveBeenCalled();
    });

    // Phase 4 (§5): identity is unbound by default; opts.inheritIdentity
    // opts into the SOURCE definition's own bound account instead, resolved
    // via ListAgentIdentitiesCommand (this flow has a definitionId, not a
    // RecentSessionRow).
    describe("opts.inheritIdentity", () => {
        it("resolves the source definition's bound account and passes it as accountId", async () => {
            listAgentIdentitiesCommand.mockResolvedValue([{ account_id: "acct-1" }]);
            await quickForkTabToNewTab("tab-1", { inheritIdentity: true });
            expect(listAgentIdentitiesCommand).toHaveBeenCalledWith(
                expect.anything(),
                { agent_id: "source-def" }
            );
            const overrides = launchAgentDefinition.mock.calls[0][1];
            expect(overrides.accountId).toBe("acct-1");
        });

        it("falls back to unbound when the source has no linked identity", async () => {
            listAgentIdentitiesCommand.mockResolvedValue([]);
            await quickForkTabToNewTab("tab-1", { inheritIdentity: true });
            const overrides = launchAgentDefinition.mock.calls[0][1];
            expect(overrides.accountId).toBe("");
        });

        it("falls back to unbound (best-effort, not thrown) when ListAgentIdentitiesCommand rejects", async () => {
            listAgentIdentitiesCommand.mockRejectedValue(new Error("boom"));
            const newTabId = await quickForkTabToNewTab("tab-1", { inheritIdentity: true });
            expect(newTabId).toBe("new-tab-1");
            const overrides = launchAgentDefinition.mock.calls[0][1];
            expect(overrides.accountId).toBe("");
        });

        it("does not resolve identity at all when inheritIdentity is not set", async () => {
            await quickForkTabToNewTab("tab-1");
            expect(listAgentIdentitiesCommand).not.toHaveBeenCalled();
        });
    });

    // Spec §4.4: a fork that can't carry history forward (provider has no
    // --fork-session equivalent) needs a visible, non-dismissable-by-accident
    // note rather than silently starting fresh. quick-fork.ts surfaces this
    // via a meta flag ForkProviderFallbackBanner (agent-view.tsx) reads.
    describe("non-Claude fallback meta flag", () => {
        it("sets the fallback meta flag when the forked provider doesn't support --fork-session and there was a session to lose", async () => {
            resolveEffectiveLaunchProvider.mockResolvedValue("codex");
            await quickForkTabToNewTab("tab-1");
            expect(setMetaCommand).toHaveBeenCalledWith(
                expect.anything(),
                { oref: "block:new-block-1", meta: { "quickfork:noHistoryFallback": true } }
            );
        });

        it("does not set the flag when the forked provider is claude", async () => {
            resolveEffectiveLaunchProvider.mockResolvedValue("claude");
            await quickForkTabToNewTab("tab-1");
            expect(setMetaCommand).not.toHaveBeenCalled();
        });

        it("does not set the flag when there was no parent session to lose in the first place", async () => {
            getObjectValue.mockReturnValue({ meta: { view: "agent", agentId: "source-def" } }); // no agent:sessionid
            resolveEffectiveLaunchProvider.mockResolvedValue("codex");
            await quickForkTabToNewTab("tab-1");
            expect(setMetaCommand).not.toHaveBeenCalled();
        });

        it("does not set the flag when launchAgentDefinition itself reports failure", async () => {
            resolveEffectiveLaunchProvider.mockResolvedValue("codex");
            launchAgentDefinition.mockResolvedValue(false);
            await quickForkTabToNewTab("tab-1");
            expect(setMetaCommand).not.toHaveBeenCalled();
        });

        it("logs but does not throw when SetMetaCommand itself rejects", async () => {
            resolveEffectiveLaunchProvider.mockResolvedValue("codex");
            setMetaCommand.mockRejectedValue(new Error("boom"));
            const newTabId = await quickForkTabToNewTab("tab-1");
            expect(newTabId).toBe("new-tab-1");
        });
    });
});
