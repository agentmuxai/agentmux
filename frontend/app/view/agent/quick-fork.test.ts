// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
// vi.mock calls below are hoisted above this import by vitest — see
// open-history-tab.test.ts's own note on why this ordering is safe despite
// not using a dynamic import.
import { quickForkAgent } from "./quick-fork";

const getNodeByBlockId = vi.fn();
const pushBlockOntoStack = vi.fn();
const closeBlockInStack = vi.fn();
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({ getNodeByBlockId }),
    pushBlockOntoStack: (...args: unknown[]) => pushBlockOntoStack(...args),
    closeBlockInStack: (...args: unknown[]) => closeBlockInStack(...args),
}));

const getObjectValue = vi.fn();
const activeTabId = vi.fn();
const pushNotification = vi.fn();
vi.mock("@/app/store/global", () => ({
    atoms: { activeTabId: () => activeTabId() },
    pushNotification: (...args: unknown[]) => pushNotification(...args),
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

const rpcCall = vi.fn();
vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: { rpcCall: (...args: unknown[]) => rpcCall(...args) },
}));

const deleteBlock = vi.fn();
vi.mock("@/app/store/services", () => ({
    ObjectService: { DeleteBlock: (...args: unknown[]) => deleteBlock(...args) },
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

vi.mock("@/util/logger", () => ({
    Logger: { warn: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

describe("quickForkAgent", () => {
    const launchAgentDefinition = vi.fn();
    const model = { blockId: "source-block", launchAgentDefinition };

    beforeEach(() => {
        vi.clearAllMocks();
        getNodeByBlockId.mockReturnValue({ id: "node-1", data: { blockStack: ["source-block"] } });
        getObjectValue.mockReturnValue({ meta: { view: "agent", agentId: "source-def", "agent:sessionid": "sid-parent" } });
        activeTabId.mockReturnValue("tab-1");
        forkAgentDefinitionCommand.mockResolvedValue({ id: "forked-def", name: "X #2", agent_type: "host" });
        rpcCall.mockResolvedValue({ block_id: "new-block-1" });
        launchAgentDefinition.mockResolvedValue(true);
        resolveEffectiveLaunchProvider.mockResolvedValue("claude");
        resolveProviderAlias.mockImplementation((id: string) => id);
        listAgentIdentitiesCommand.mockResolvedValue([]);
        setMetaCommand.mockResolvedValue(undefined);
        closeBlockInStack.mockResolvedValue(undefined);
        deleteBlock.mockResolvedValue(undefined);
    });

    afterEach(() => {
        vi.restoreAllMocks();
    });

    it("returns false when the pane has no live agent to fork", async () => {
        getObjectValue.mockReturnValue(undefined);
        expect(await quickForkAgent(model)).toBe(false);
        expect(forkAgentDefinitionCommand).not.toHaveBeenCalled();
    });

    it("returns false when the pane's own layout node can't be found", async () => {
        getNodeByBlockId.mockReturnValue(undefined);
        expect(await quickForkAgent(model)).toBe(false);
        expect(forkAgentDefinitionCommand).not.toHaveBeenCalled();
    });

    it("forks the definition, pushes the new block onto THIS pane's own stack (not a new window tab), and launches with history carryover + unbound identity", async () => {
        const result = await quickForkAgent(model);

        expect(forkAgentDefinitionCommand).toHaveBeenCalledWith(
            expect.anything(),
            { source_id: "source-def", branch_label: "" },
        );
        // pane.open with skip_placement — same primitive Agent History / the
        // "+" new-tab button use — NOT a WorkspaceService.CreateTab call.
        expect(rpcCall).toHaveBeenCalledWith(
            "pane.open",
            { view: "agent", skip_placement: true, tab_id: "tab-1", meta: { view: "agent" } },
            {},
        );
        expect(pushBlockOntoStack).toHaveBeenCalledWith(expect.anything(), "node-1", "new-block-1");
        expect(launchAgentDefinition).toHaveBeenCalledTimes(1);
        const [forkedDef, overrides, targetBlockId, targetTabId] = launchAgentDefinition.mock.calls[0];
        expect(forkedDef).toEqual({ id: "forked-def", name: "X #2", agent_type: "host" });
        expect(overrides.continueSessionId).toBe("sid-parent");
        expect(overrides.forkSession).toBe(true);
        expect(overrides.accountId).toBe("");
        expect(targetBlockId).toBe("new-block-1");
        expect(targetTabId).toBe("tab-1");
        expect(result).toBe(true);
    });

    // Codex's review of PR #2746: the several RPCs this function awaits
    // (fork, identity lookup, pane.open, launch) leave a window for the
    // user to switch window tabs mid-flight. pane.open's skip_placement
    // path and launchAgentDefinition's ControllerResyncCommand would
    // otherwise each independently resolve "the active tab" at their own
    // execution time (possibly AFTER the switch), silently registering the
    // new block under the wrong tab. The active tab id must be captured
    // ONCE, synchronously, before any await, and threaded through both.
    it("captures the active tab id synchronously and passes the SAME value to both pane.open and launchAgentDefinition, even if the active tab changes mid-flight", async () => {
        activeTabId.mockReturnValue("tab-original");
        let resolveForkCommand: (v: unknown) => void;
        forkAgentDefinitionCommand.mockReturnValue(
            new Promise((resolve) => { resolveForkCommand = resolve; }),
        );

        const pending = quickForkAgent(model);
        // Simulate the user switching window tabs while the fork RPC is
        // still in flight — a later read of activeTabId() must NOT affect
        // this in-progress call.
        activeTabId.mockReturnValue("tab-switched-to");
        resolveForkCommand!({ id: "forked-def", name: "X #2", agent_type: "host" });
        await pending;

        expect(rpcCall).toHaveBeenCalledWith(
            "pane.open",
            expect.objectContaining({ tab_id: "tab-original" }),
            {},
        );
        const [, , , targetTabId] = launchAgentDefinition.mock.calls[0];
        expect(targetTabId).toBe("tab-original");
    });

    it("returns the launch result even when launchAgentDefinition itself reports failure (best-effort logging, not a thrown error)", async () => {
        launchAgentDefinition.mockResolvedValue(false);
        expect(await quickForkAgent(model)).toBe(false);
    });

    // Codex P2 on PR #2746: a failed launch previously left the new block
    // pushed onto the stack and active, stranding the user on a blank/
    // broken pane-stack tab with no visible way to recover.
    it("pops the new block back out of the stack and returns false when launchAgentDefinition reports failure", async () => {
        launchAgentDefinition.mockResolvedValue(false);
        expect(await quickForkAgent(model)).toBe(false);
        expect(closeBlockInStack).toHaveBeenCalledWith(expect.anything(), "node-1", "new-block-1");
    });

    it("does not push closeBlockInStack when the launch succeeds", async () => {
        await quickForkAgent(model);
        expect(closeBlockInStack).not.toHaveBeenCalled();
    });

    // Codex's second-round review of PR #2746: the cleanup above is
    // otherwise silent — without a notification, the fork just flashes in
    // and back out with no indication anything went wrong.
    it("pushes a failure notification when launchAgentDefinition reports failure", async () => {
        launchAgentDefinition.mockResolvedValue(false);
        await quickForkAgent(model);
        expect(pushNotification).toHaveBeenCalledWith(
            expect.objectContaining({ type: "error", title: "Quick-fork failed" }),
        );
    });

    it("does not push a failure notification when the launch succeeds", async () => {
        await quickForkAgent(model);
        expect(pushNotification).not.toHaveBeenCalled();
    });

    it("logs but does not throw when closeBlockInStack itself rejects on a failed launch", async () => {
        launchAgentDefinition.mockResolvedValue(false);
        closeBlockInStack.mockRejectedValue(new Error("boom"));
        expect(await quickForkAgent(model)).toBe(false);
    });

    it("returns false and pushes a notification if ForkAgentDefinitionCommand rejects", async () => {
        forkAgentDefinitionCommand.mockRejectedValue(new Error("boom"));
        expect(await quickForkAgent(model)).toBe(false);
        expect(launchAgentDefinition).not.toHaveBeenCalled();
        // No block was ever created at this point — nothing to clean up.
        expect(closeBlockInStack).not.toHaveBeenCalled();
        expect(deleteBlock).not.toHaveBeenCalled();
    });

    // Codex's third-round review of PR #2746: launchAgentDefinition's own
    // doc comment claims it never throws, but several of its awaited calls
    // (resolveEffectiveLaunchProvider, checkNodejsForProvider, ensureAuthDir)
    // run unguarded before its own internal try/catches — so it CAN reject,
    // not just resolve to false, and by then the block has already been
    // pushed onto the stack.
    it("pops the new block back out of the stack when launchAgentDefinition itself REJECTS (not just resolves false)", async () => {
        launchAgentDefinition.mockRejectedValue(new Error("boom"));
        expect(await quickForkAgent(model)).toBe(false);
        expect(closeBlockInStack).toHaveBeenCalledWith(expect.anything(), "node-1", "new-block-1");
        expect(pushNotification).toHaveBeenCalledWith(
            expect.objectContaining({ type: "error", title: "Quick-fork failed" }),
        );
    });

    it("deletes the orphaned block directly (not closeBlockInStack) if the source pane is ALSO gone when launchAgentDefinition rejects", async () => {
        launchAgentDefinition.mockRejectedValue(new Error("boom"));
        getNodeByBlockId
            .mockReturnValueOnce({ id: "node-1", data: { blockStack: ["source-block"] } }) // initial check
            .mockReturnValueOnce({ id: "node-1", data: { blockStack: ["source-block"] } }) // pre-push re-check
            .mockReturnValueOnce(undefined); // catch-block cleanup lookup
        expect(await quickForkAgent(model)).toBe(false);
        expect(closeBlockInStack).not.toHaveBeenCalled();
        expect(deleteBlock).toHaveBeenCalledWith("new-block-1");
    });

    it("deletes the orphaned skip_placement block and returns false if the pane closed while the RPCs were in flight", async () => {
        getNodeByBlockId
            .mockReturnValueOnce({ id: "node-1", data: { blockStack: ["source-block"] } }) // initial check
            .mockReturnValueOnce(undefined); // re-check after pane.open
        expect(await quickForkAgent(model)).toBe(false);
        expect(deleteBlock).toHaveBeenCalledWith("new-block-1");
        expect(pushBlockOntoStack).not.toHaveBeenCalled();
        expect(launchAgentDefinition).not.toHaveBeenCalled();
    });

    describe("opts.inheritIdentity", () => {
        it("resolves the source definition's bound account for the fork's own provider and passes it as accountId", async () => {
            listAgentIdentitiesCommand.mockResolvedValue([{ account_id: "acct-1", provider: "claude" }]);
            await quickForkAgent(model, { inheritIdentity: true });
            expect(listAgentIdentitiesCommand).toHaveBeenCalledWith(
                expect.anything(),
                { agent_id: "source-def" },
            );
            const overrides = launchAgentDefinition.mock.calls[0][1];
            expect(overrides.accountId).toBe("acct-1");
        });

        it("filters to the link matching the fork's own effective provider, ignoring other-provider links", async () => {
            listAgentIdentitiesCommand.mockResolvedValue([
                { account_id: "github-acct", provider: "github" },
                { account_id: "claude-acct", provider: "claude" },
            ]);
            await quickForkAgent(model, { inheritIdentity: true });
            const overrides = launchAgentDefinition.mock.calls[0][1];
            expect(overrides.accountId).toBe("claude-acct");
        });

        it("picks the LAST canonical-equivalent row when both a canonical and legacy-alias row exist, not the first", async () => {
            listAgentIdentitiesCommand.mockResolvedValue([
                { account_id: "acct-alias", provider: "claude-code" },
                { account_id: "acct-canonical", provider: "claude" },
            ]);
            await quickForkAgent(model, { inheritIdentity: true });
            const overrides = launchAgentDefinition.mock.calls[0][1];
            expect(overrides.accountId).toBe("acct-canonical");
        });

        it("falls back to unbound (best-effort, not thrown) when ListAgentIdentitiesCommand rejects", async () => {
            listAgentIdentitiesCommand.mockRejectedValue(new Error("boom"));
            const result = await quickForkAgent(model, { inheritIdentity: true });
            expect(result).toBe(true);
            const overrides = launchAgentDefinition.mock.calls[0][1];
            expect(overrides.accountId).toBe("");
        });

        it("does not resolve identity at all when inheritIdentity is not set", async () => {
            await quickForkAgent(model);
            expect(listAgentIdentitiesCommand).not.toHaveBeenCalled();
        });
    });

    describe("non-Claude fallback meta flag", () => {
        it("sets the fallback meta flag when the forked provider doesn't support --fork-session and there was a session to lose", async () => {
            resolveEffectiveLaunchProvider.mockResolvedValue("codex");
            await quickForkAgent(model);
            expect(setMetaCommand).toHaveBeenCalledWith(
                expect.anything(),
                { oref: "block:new-block-1", meta: { "quickfork:noHistoryFallback": true } },
            );
        });

        it("does not set the flag when the forked provider is claude", async () => {
            resolveEffectiveLaunchProvider.mockResolvedValue("claude");
            await quickForkAgent(model);
            expect(setMetaCommand).not.toHaveBeenCalled();
        });

        it("does not set the flag when there was no parent session to lose in the first place", async () => {
            getObjectValue.mockReturnValue({ meta: { view: "agent", agentId: "source-def" } });
            resolveEffectiveLaunchProvider.mockResolvedValue("codex");
            await quickForkAgent(model);
            expect(setMetaCommand).not.toHaveBeenCalled();
        });

        it("does not set the flag when launchAgentDefinition itself reports failure", async () => {
            resolveEffectiveLaunchProvider.mockResolvedValue("codex");
            launchAgentDefinition.mockResolvedValue(false);
            await quickForkAgent(model);
            expect(setMetaCommand).not.toHaveBeenCalled();
        });

        it("logs but does not throw when SetMetaCommand itself rejects", async () => {
            resolveEffectiveLaunchProvider.mockResolvedValue("codex");
            setMetaCommand.mockRejectedValue(new Error("boom"));
            const result = await quickForkAgent(model);
            expect(result).toBe(true);
        });
    });
});
