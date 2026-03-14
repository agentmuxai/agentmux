// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { waveEventSubscribe } from "@/app/store/wps";
import { createSignal, type Accessor, type Setter } from "solid-js";

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

    viewIcon: Accessor<string> = () => "hammer";
    viewName: Accessor<string> = () => "Forge";
    viewText: Accessor<string | HeaderElem[]> = () => [];
    noPadding: Accessor<boolean> = () => false;

    get viewComponent(): ViewComponent {
        return null; // set by the forge barrel to avoid circular import
    }

    // UI state
    private _view = createSignal<ForgeView>("list");
    viewAtom: Accessor<ForgeView> = this._view[0];
    private setView: Setter<ForgeView> = this._view[1];

    private _agents = createSignal<ForgeAgent[]>([]);
    agentsAtom: Accessor<ForgeAgent[]> = this._agents[0];
    private setAgents: Setter<ForgeAgent[]> = this._agents[1];

    private _editingAgent = createSignal<ForgeAgent | null>(null);
    editingAgentAtom: Accessor<ForgeAgent | null> = this._editingAgent[0];
    private setEditingAgent: Setter<ForgeAgent | null> = this._editingAgent[1];

    private _loading = createSignal<boolean>(false);
    loadingAtom: Accessor<boolean> = this._loading[0];
    private setLoading: Setter<boolean> = this._loading[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    private setError: Setter<string | null> = this._error[1];

    // Detail view state
    private _detailAgent = createSignal<ForgeAgent | null>(null);
    detailAgentAtom: Accessor<ForgeAgent | null> = this._detailAgent[0];
    private setDetailAgent: Setter<ForgeAgent | null> = this._detailAgent[1];

    private _content = createSignal<Record<string, ForgeContent>>({});
    contentAtom: Accessor<Record<string, ForgeContent>> = this._content[0];
    private setContent: Setter<Record<string, ForgeContent>> = this._content[1];

    private _activeTab = createSignal<ContentTabId>("soul");
    activeTabAtom: Accessor<ContentTabId> = this._activeTab[0];
    setActiveTab: Setter<ContentTabId> = this._activeTab[1];

    private _activeSection = createSignal<DetailSection>("content");
    activeSectionAtom: Accessor<DetailSection> = this._activeSection[0];
    setActiveSection: Setter<DetailSection> = this._activeSection[1];

    private _contentLoading = createSignal<boolean>(false);
    contentLoadingAtom: Accessor<boolean> = this._contentLoading[0];
    private setContentLoading: Setter<boolean> = this._contentLoading[1];

    private _contentSaving = createSignal<boolean>(false);
    contentSavingAtom: Accessor<boolean> = this._contentSaving[0];
    private setContentSaving: Setter<boolean> = this._contentSaving[1];

    // Skills state
    private _skills = createSignal<ForgeSkill[]>([]);
    skillsAtom: Accessor<ForgeSkill[]> = this._skills[0];
    private setSkills: Setter<ForgeSkill[]> = this._skills[1];

    private _editingSkill = createSignal<ForgeSkill | null>(null);
    editingSkillAtom: Accessor<ForgeSkill | null> = this._editingSkill[0];
    setEditingSkill: Setter<ForgeSkill | null> = this._editingSkill[1];

    private _skillsLoading = createSignal<boolean>(false);
    skillsLoadingAtom: Accessor<boolean> = this._skillsLoading[0];
    private setSkillsLoading: Setter<boolean> = this._skillsLoading[1];

    // History state
    private _history = createSignal<ForgeHistory[]>([]);
    historyAtom: Accessor<ForgeHistory[]> = this._history[0];
    private setHistory: Setter<ForgeHistory[]> = this._history[1];

    private _historyLoading = createSignal<boolean>(false);
    historyLoadingAtom: Accessor<boolean> = this._historyLoading[0];
    private setHistoryLoading: Setter<boolean> = this._historyLoading[1];

    private _historySearch = createSignal<string>("");
    historySearchAtom: Accessor<string> = this._historySearch[0];
    private setHistorySearch: Setter<string> = this._historySearch[1];

    // Import state
    private _importing = createSignal<boolean>(false);
    importingAtom: Accessor<boolean> = this._importing[0];
    private setImporting: Setter<boolean> = this._importing[1];

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
            this.setAgents(agents ?? []);
        } catch {
            // silently ignore on load
        }
    };

    createAgent = async (data: CommandCreateForgeAgentData): Promise<void> => {
        this.setLoading(true);
        this.setError(null);
        try {
            await RpcApi.CreateForgeAgentCommand(TabRpcClient, data);
            this.setView("list");
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
        } finally {
            this.setLoading(false);
        }
    };

    updateAgent = async (data: CommandUpdateForgeAgentData): Promise<void> => {
        this.setLoading(true);
        this.setError(null);
        try {
            await RpcApi.UpdateForgeAgentCommand(TabRpcClient, data);
            this.setView("list");
            this.setEditingAgent(null);
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
        } finally {
            this.setLoading(false);
        }
    };

    deleteAgent = async (id: string): Promise<void> => {
        try {
            await RpcApi.DeleteForgeAgentCommand(TabRpcClient, { id });
        } catch {
            // silently ignore
        }
    };

    startCreate = (): void => {
        this.setEditingAgent(null);
        this.setError(null);
        this.setView("create");
    };

    startEdit = (agent: ForgeAgent): void => {
        this.setEditingAgent(agent);
        this.setError(null);
        this.setView("edit");
    };

    cancelForm = (): void => {
        this.setEditingAgent(null);
        this.setError(null);
        this.setView("list");
    };

    // ── Detail view methods ──────────────────────────────────────────────

    openDetail = async (agent: ForgeAgent): Promise<void> => {
        this.setDetailAgent(agent);
        this.setActiveTab("soul");
        this.setActiveSection("content");
        this.setContent({});
        this.setSkills([]);
        this.setHistory([]);
        this.setView("detail");
        await this.loadContent(agent.id);
    };

    closeDetail = (): void => {
        this.setDetailAgent(null);
        this.setContent({});
        this.setSkills([]);
        this.setHistory([]);
        this.setView("list");
    };

    loadContent = async (agentId: string): Promise<void> => {
        this.setContentLoading(true);
        try {
            const contents = await RpcApi.GetAllForgeContentCommand(TabRpcClient, { agent_id: agentId });
            const map: Record<string, ForgeContent> = {};
            for (const c of contents ?? []) {
                map[c.content_type] = c;
            }
            this.setContent(map);
        } catch {
            // silently ignore
        } finally {
            this.setContentLoading(false);
        }
    };

    saveContent = async (agentId: string, contentType: string, content: string): Promise<void> => {
        this.setContentSaving(true);
        try {
            const result = await RpcApi.SetForgeContentCommand(TabRpcClient, {
                agent_id: agentId,
                content_type: contentType,
                content,
            });
            // Update local cache
            const current = this.contentAtom();
            this.setContent({
                ...current,
                [contentType]: result ?? { agent_id: agentId, content_type: contentType, content, updated_at: Date.now() },
            });
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
        } finally {
            this.setContentSaving(false);
        }
    };

    private reloadContentIfDetail = async (): Promise<void> => {
        const view = this.viewAtom();
        const agent = this.detailAgentAtom();
        if (view === "detail" && agent) {
            await this.loadContent(agent.id);
        }
    };

    // ── Skills methods ──────────────────────────────────────────────────

    loadSkills = async (agentId: string): Promise<void> => {
        this.setSkillsLoading(true);
        try {
            const skills = await RpcApi.ListForgeSkillsCommand(TabRpcClient, { agent_id: agentId });
            this.setSkills(skills ?? []);
        } catch {
            // silently ignore
        } finally {
            this.setSkillsLoading(false);
        }
    };

    createSkill = async (data: CommandCreateForgeSkillData): Promise<void> => {
        this.setError(null);
        try {
            await RpcApi.CreateForgeSkillCommand(TabRpcClient, data);
            this.setEditingSkill(null);
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
        }
    };

    updateSkill = async (data: CommandUpdateForgeSkillData): Promise<void> => {
        this.setError(null);
        try {
            await RpcApi.UpdateForgeSkillCommand(TabRpcClient, data);
            this.setEditingSkill(null);
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
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
        const view = this.viewAtom();
        const agent = this.detailAgentAtom();
        if (view === "detail" && agent) {
            await this.loadSkills(agent.id);
        }
    };

    // ── History methods ──────────────────────────────────────────────────

    loadHistory = async (agentId: string, sessionDate?: string): Promise<void> => {
        this.setHistoryLoading(true);
        try {
            const entries = await RpcApi.ListForgeHistoryCommand(TabRpcClient, {
                agent_id: agentId,
                session_date: sessionDate,
                limit: 100,
            });
            this.setHistory(entries ?? []);
        } catch {
            // silently ignore
        } finally {
            this.setHistoryLoading(false);
        }
    };

    searchHistory = async (agentId: string, query: string): Promise<void> => {
        this.setHistoryLoading(true);
        try {
            const entries = await RpcApi.SearchForgeHistoryCommand(TabRpcClient, {
                agent_id: agentId,
                query,
                limit: 100,
            });
            this.setHistory(entries ?? []);
        } catch {
            // silently ignore
        } finally {
            this.setHistoryLoading(false);
        }
    };

    private reloadHistoryIfDetail = async (): Promise<void> => {
        const view = this.viewAtom();
        const agent = this.detailAgentAtom();
        const section = this.activeSectionAtom();
        if (view === "detail" && agent && section === "history") {
            await this.loadHistory(agent.id);
        }
    };

    // ── Import from Claw ──────────────────────────────────────────────────

    importFromClaw = async (workspacePath: string, agentName: string): Promise<void> => {
        this.setImporting(true);
        this.setError(null);
        try {
            await RpcApi.ImportForgeFromClawCommand(TabRpcClient, {
                workspace_path: workspacePath,
                agent_name: agentName,
            });
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
        } finally {
            this.setImporting(false);
        }
    };

    // ── Edit from detail ──────────────────────────────────────────────────

    startEditFromDetail = (): void => {
        const agent = this.detailAgentAtom();
        if (agent) {
            this.setEditingAgent(agent);
            this.setError(null);
            this.setView("edit");
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
