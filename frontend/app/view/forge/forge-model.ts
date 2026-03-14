// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { waveEventSubscribe } from "@/app/store/wps";
import { atom, PrimitiveAtom } from "jotai";

export type ForgeView = "list" | "create" | "edit" | "detail";

export const CONTENT_TABS = ["soul", "agentmd", "mcp", "env"] as const;
export type ContentTabId = (typeof CONTENT_TABS)[number];

export type DetailSection = "content" | "skills" | "history";

export const CONTENT_TAB_LABELS: Record<ContentTabId, string> = {
    soul: "Soul",
    agentmd: "Instructions",
    mcp: "MCP",
    env: "Env",
};

export const SKILL_TYPES = ["prompt", "command", "workflow", "mcp-tool"] as const;
export type SkillType = (typeof SKILL_TYPES)[number];

export class ForgeViewModel implements ViewModel {
    viewType = "forge";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon = atom("hammer");
    viewName = atom("Forge");
    viewText = atom<string | HeaderElem[]>([]);
    noPadding = atom(false);

    get viewComponent(): ViewComponent {
        return null; // set by the forge barrel to avoid circular import
    }

    // UI state
    viewAtom: PrimitiveAtom<ForgeView> = atom<ForgeView>("list");
    agentsAtom: PrimitiveAtom<ForgeAgent[]> = atom<ForgeAgent[]>([]);
    editingAgentAtom: PrimitiveAtom<ForgeAgent | null> = atom<ForgeAgent | null>(null);
    loadingAtom: PrimitiveAtom<boolean> = atom(false);
    errorAtom: PrimitiveAtom<string | null> = atom<string | null>(null);

    // Detail view state
    detailAgentAtom: PrimitiveAtom<ForgeAgent | null> = atom<ForgeAgent | null>(null);
    contentAtom: PrimitiveAtom<Record<string, ForgeContent>> = atom<Record<string, ForgeContent>>({});
    activeTabAtom: PrimitiveAtom<ContentTabId> = atom<ContentTabId>("soul");
    activeSectionAtom: PrimitiveAtom<DetailSection> = atom<DetailSection>("content");
    contentLoadingAtom: PrimitiveAtom<boolean> = atom(false);
    contentSavingAtom: PrimitiveAtom<boolean> = atom(false);

    // Skills state
    skillsAtom: PrimitiveAtom<ForgeSkill[]> = atom<ForgeSkill[]>([]);
    editingSkillAtom: PrimitiveAtom<ForgeSkill | null> = atom<ForgeSkill | null>(null);
    skillsLoadingAtom: PrimitiveAtom<boolean> = atom(false);

    // History state
    historyAtom: PrimitiveAtom<ForgeHistory[]> = atom<ForgeHistory[]>([]);
    historyLoadingAtom: PrimitiveAtom<boolean> = atom(false);
    historySearchAtom: PrimitiveAtom<string> = atom("");

    // Import state
    importingAtom: PrimitiveAtom<boolean> = atom(false);

    private unsubForgeChanged: (() => void) | null = null;
    private unsubContentChanged: (() => void) | null = null;
    private unsubSkillsChanged: (() => void) | null = null;
    private unsubHistoryChanged: (() => void) | null = null;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.loadAgents();
        this.unsubForgeChanged = waveEventSubscribe({
            eventType: "forgeagents:changed",
            handler: () => this.loadAgents(),
        });
        this.unsubContentChanged = waveEventSubscribe({
            eventType: "forgecontent:changed",
            handler: () => this.reloadContentIfDetail(),
        });
        this.unsubSkillsChanged = waveEventSubscribe({
            eventType: "forgeskills:changed",
            handler: () => this.reloadSkillsIfDetail(),
        });
        this.unsubHistoryChanged = waveEventSubscribe({
            eventType: "forgehistory:changed",
            handler: () => this.reloadHistoryIfDetail(),
        });
    }

    loadAgents = async (): Promise<void> => {
        try {
            const agents = await RpcApi.ListForgeAgentsCommand(TabRpcClient);
            const { globalStore } = await import("@/app/store/global");
            globalStore.set(this.agentsAtom, agents ?? []);
        } catch {
            // silently ignore on load
        }
    };

    createAgent = async (data: CommandCreateForgeAgentData): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.loadingAtom, true);
        globalStore.set(this.errorAtom, null);
        try {
            await RpcApi.CreateForgeAgentCommand(TabRpcClient, data);
            globalStore.set(this.viewAtom, "list");
        } catch (e: any) {
            globalStore.set(this.errorAtom, String(e?.message ?? e));
        } finally {
            globalStore.set(this.loadingAtom, false);
        }
    };

    updateAgent = async (data: CommandUpdateForgeAgentData): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.loadingAtom, true);
        globalStore.set(this.errorAtom, null);
        try {
            await RpcApi.UpdateForgeAgentCommand(TabRpcClient, data);
            globalStore.set(this.viewAtom, "list");
            globalStore.set(this.editingAgentAtom, null);
        } catch (e: any) {
            globalStore.set(this.errorAtom, String(e?.message ?? e));
        } finally {
            globalStore.set(this.loadingAtom, false);
        }
    };

    deleteAgent = async (id: string): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        try {
            await RpcApi.DeleteForgeAgentCommand(TabRpcClient, { id });
        } catch {
            // silently ignore
        }
    };

    startCreate = async (): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.editingAgentAtom, null);
        globalStore.set(this.errorAtom, null);
        globalStore.set(this.viewAtom, "create");
    };

    startEdit = async (agent: ForgeAgent): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.editingAgentAtom, agent);
        globalStore.set(this.errorAtom, null);
        globalStore.set(this.viewAtom, "edit");
    };

    cancelForm = async (): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.editingAgentAtom, null);
        globalStore.set(this.errorAtom, null);
        globalStore.set(this.viewAtom, "list");
    };

    // ── Detail view methods ──────────────────────────────────────────────

    openDetail = async (agent: ForgeAgent): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.detailAgentAtom, agent);
        globalStore.set(this.activeTabAtom, "soul");
        globalStore.set(this.activeSectionAtom, "content");
        globalStore.set(this.contentAtom, {});
        globalStore.set(this.skillsAtom, []);
        globalStore.set(this.historyAtom, []);
        globalStore.set(this.viewAtom, "detail");
        await this.loadContent(agent.id);
    };

    closeDetail = async (): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.detailAgentAtom, null);
        globalStore.set(this.contentAtom, {});
        globalStore.set(this.skillsAtom, []);
        globalStore.set(this.historyAtom, []);
        globalStore.set(this.viewAtom, "list");
    };

    loadContent = async (agentId: string): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.contentLoadingAtom, true);
        try {
            const contents = await RpcApi.GetAllForgeContentCommand(TabRpcClient, { agent_id: agentId });
            const map: Record<string, ForgeContent> = {};
            for (const c of contents ?? []) {
                map[c.content_type] = c;
            }
            globalStore.set(this.contentAtom, map);
        } catch {
            // silently ignore
        } finally {
            globalStore.set(this.contentLoadingAtom, false);
        }
    };

    saveContent = async (agentId: string, contentType: string, content: string): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.contentSavingAtom, true);
        try {
            const result = await RpcApi.SetForgeContentCommand(TabRpcClient, {
                agent_id: agentId,
                content_type: contentType,
                content,
            });
            // Update local cache
            const current = globalStore.get(this.contentAtom);
            globalStore.set(this.contentAtom, {
                ...current,
                [contentType]: result ?? { agent_id: agentId, content_type: contentType, content, updated_at: Date.now() },
            });
        } catch (e: any) {
            globalStore.set(this.errorAtom, String(e?.message ?? e));
        } finally {
            globalStore.set(this.contentSavingAtom, false);
        }
    };

    private reloadContentIfDetail = async (): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        const view = globalStore.get(this.viewAtom);
        const agent = globalStore.get(this.detailAgentAtom);
        if (view === "detail" && agent) {
            await this.loadContent(agent.id);
        }
    };

    // ── Skills methods ──────────────────────────────────────────────────

    loadSkills = async (agentId: string): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.skillsLoadingAtom, true);
        try {
            const skills = await RpcApi.ListForgeSkillsCommand(TabRpcClient, { agent_id: agentId });
            globalStore.set(this.skillsAtom, skills ?? []);
        } catch {
            // silently ignore
        } finally {
            globalStore.set(this.skillsLoadingAtom, false);
        }
    };

    createSkill = async (data: CommandCreateForgeSkillData): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.errorAtom, null);
        try {
            await RpcApi.CreateForgeSkillCommand(TabRpcClient, data);
            globalStore.set(this.editingSkillAtom, null);
        } catch (e: any) {
            globalStore.set(this.errorAtom, String(e?.message ?? e));
        }
    };

    updateSkill = async (data: CommandUpdateForgeSkillData): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.errorAtom, null);
        try {
            await RpcApi.UpdateForgeSkillCommand(TabRpcClient, data);
            globalStore.set(this.editingSkillAtom, null);
        } catch (e: any) {
            globalStore.set(this.errorAtom, String(e?.message ?? e));
        }
    };

    deleteSkill = async (id: string): Promise<void> => {
        try {
            await RpcApi.DeleteForgeSkillCommand(TabRpcClient, { id });
        } catch {
            // silently ignore
        }
    };

    private reloadSkillsIfDetail = async (): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        const view = globalStore.get(this.viewAtom);
        const agent = globalStore.get(this.detailAgentAtom);
        if (view === "detail" && agent) {
            await this.loadSkills(agent.id);
        }
    };

    // ── History methods ──────────────────────────────────────────────────

    loadHistory = async (agentId: string, sessionDate?: string): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.historyLoadingAtom, true);
        try {
            const entries = await RpcApi.ListForgeHistoryCommand(TabRpcClient, {
                agent_id: agentId,
                session_date: sessionDate,
                limit: 100,
            });
            globalStore.set(this.historyAtom, entries ?? []);
        } catch {
            // silently ignore
        } finally {
            globalStore.set(this.historyLoadingAtom, false);
        }
    };

    searchHistory = async (agentId: string, query: string): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.historyLoadingAtom, true);
        try {
            const entries = await RpcApi.SearchForgeHistoryCommand(TabRpcClient, {
                agent_id: agentId,
                query,
                limit: 100,
            });
            globalStore.set(this.historyAtom, entries ?? []);
        } catch {
            // silently ignore
        } finally {
            globalStore.set(this.historyLoadingAtom, false);
        }
    };

    private reloadHistoryIfDetail = async (): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        const view = globalStore.get(this.viewAtom);
        const agent = globalStore.get(this.detailAgentAtom);
        const section = globalStore.get(this.activeSectionAtom);
        if (view === "detail" && agent && section === "history") {
            await this.loadHistory(agent.id);
        }
    };

    // ── Import from Claw ──────────────────────────────────────────────────

    importFromClaw = async (workspacePath: string, agentName: string): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.importingAtom, true);
        globalStore.set(this.errorAtom, null);
        try {
            await RpcApi.ImportForgeFromClawCommand(TabRpcClient, {
                workspace_path: workspacePath,
                agent_name: agentName,
            });
        } catch (e: any) {
            globalStore.set(this.errorAtom, String(e?.message ?? e));
        } finally {
            globalStore.set(this.importingAtom, false);
        }
    };

    // ── Edit from detail ──────────────────────────────────────────────────

    startEditFromDetail = async (): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        const agent = globalStore.get(this.detailAgentAtom);
        if (agent) {
            globalStore.set(this.editingAgentAtom, agent);
            globalStore.set(this.errorAtom, null);
            globalStore.set(this.viewAtom, "edit");
        }
    };

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {
        this.unsubForgeChanged?.();
        this.unsubContentChanged?.();
        this.unsubSkillsChanged?.();
        this.unsubHistoryChanged?.();
    }
}
