// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// NOTE: the agent-definition view is NOT a standalone pane. It is embedded
// inside the agent pane as a floating panel (AgentCardSettingsPanel →
// AgentDefDetail / AgentDefForm). The standalone forge widget was removed in
// v0.33.197. Do not re-register this as a block view — agent configuration
// lives inside the agent pane.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { createSignal, type Accessor, type Setter } from "solid-js";

export type AgentDefView = "list" | "create" | "edit" | "detail";

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

export class AgentDefViewModel implements ViewModel {
    viewType = "agent-def";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "hammer";
    viewName: Accessor<string> = () => "Agents";
    viewText: Accessor<string | HeaderElem[]> = () => [];
    noPadding: Accessor<boolean> = () => false;

    get viewComponent(): ViewComponent {
        return null; // set by the forge barrel to avoid circular import
    }

    // UI state
    private _view = createSignal<AgentDefView>("list");
    viewAtom: Accessor<AgentDefView> = this._view[0];
    private setView: Setter<AgentDefView> = this._view[1];

    private _agents = createSignal<AgentDefinition[]>([]);
    agentsAtom: Accessor<AgentDefinition[]> = this._agents[0];
    private setAgents: Setter<AgentDefinition[]> = this._agents[1];

    private _editingAgent = createSignal<AgentDefinition | null>(null);
    editingAgentAtom: Accessor<AgentDefinition | null> = this._editingAgent[0];
    private setEditingAgent: Setter<AgentDefinition | null> = this._editingAgent[1];

    private _loading = createSignal<boolean>(false);
    loadingAtom: Accessor<boolean> = this._loading[0];
    private setLoading: Setter<boolean> = this._loading[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    private setError: Setter<string | null> = this._error[1];

    // Detail view state
    private _detailAgent = createSignal<AgentDefinition | null>(null);
    detailAgentAtom: Accessor<AgentDefinition | null> = this._detailAgent[0];
    private setDetailAgent: Setter<AgentDefinition | null> = this._detailAgent[1];

    private _content = createSignal<Record<string, AgentContent>>({});
    contentAtom: Accessor<Record<string, AgentContent>> = this._content[0];
    private setContent: Setter<Record<string, AgentContent>> = this._content[1];

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
    private _skills = createSignal<AgentSkill[]>([]);
    skillsAtom: Accessor<AgentSkill[]> = this._skills[0];
    private setSkills: Setter<AgentSkill[]> = this._skills[1];

    private _editingSkill = createSignal<AgentSkill | null>(null);
    editingSkillAtom: Accessor<AgentSkill | null> = this._editingSkill[0];
    setEditingSkill: Setter<AgentSkill | null> = this._editingSkill[1];

    private _skillsLoading = createSignal<boolean>(false);
    skillsLoadingAtom: Accessor<boolean> = this._skillsLoading[0];
    private setSkillsLoading: Setter<boolean> = this._skillsLoading[1];

    // History state
    private _history = createSignal<AgentHistory[]>([]);
    historyAtom: Accessor<AgentHistory[]> = this._history[0];
    private setHistory: Setter<AgentHistory[]> = this._history[1];

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

    private unsubAgentDefChanged: (() => void) | null = null;
    private unsubContentChanged: (() => void) | null = null;
    private unsubSkillsChanged: (() => void) | null = null;
    private unsubHistoryChanged: (() => void) | null = null;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.loadAgents();
        this.unsubAgentDefChanged = waveEventSubscribe({
            eventType: "agents:changed",
            handler: () => this.loadAgents(),
        });
        this.unsubContentChanged = waveEventSubscribe({
            eventType: "agentcontent:changed",
            handler: () => this.reloadContentIfDetail(),
        });
        this.unsubSkillsChanged = waveEventSubscribe({
            eventType: "agentskills:changed",
            handler: () => this.reloadSkillsIfDetail(),
        });
        this.unsubHistoryChanged = waveEventSubscribe({
            eventType: "agenthistory:changed",
            handler: () => this.reloadHistoryIfDetail(),
        });
    }

    loadAgents = async (): Promise<void> => {
        try {
            const agents = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient);
            this.setAgents(agents ?? []);
        } catch {
            // silently ignore on load
        }
    };

    createAgent = async (data: CommandCreateAgentDefinitionData): Promise<void> => {
        this.setLoading(true);
        this.setError(null);
        try {
            await RpcApi.CreateAgentDefinitionCommand(TabRpcClient, data);
            this.setView("list");
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
        } finally {
            this.setLoading(false);
        }
    };

    updateAgent = async (data: CommandUpdateAgentDefinitionData): Promise<void> => {
        this.setLoading(true);
        this.setError(null);
        try {
            await RpcApi.UpdateAgentDefinitionCommand(TabRpcClient, data);
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
            await RpcApi.DeleteAgentDefinitionCommand(TabRpcClient, { id });
        } catch {
            // silently ignore
        }
    };

    startCreate = (): void => {
        this.setEditingAgent(null);
        this.setError(null);
        this.setView("create");
    };

    startEdit = (agent: AgentDefinition): void => {
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

    openDetail = async (agent: AgentDefinition): Promise<void> => {
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
            const contents = await RpcApi.GetAllAgentContentCommand(TabRpcClient, { agent_id: agentId });
            const map: Record<string, AgentContent> = {};
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
            const result = await RpcApi.SetAgentContentCommand(TabRpcClient, {
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
            const skills = await RpcApi.ListAgentSkillsCommand(TabRpcClient, { agent_id: agentId });
            this.setSkills(skills ?? []);
        } catch {
            // silently ignore
        } finally {
            this.setSkillsLoading(false);
        }
    };

    createSkill = async (data: CommandCreateAgentSkillData): Promise<void> => {
        this.setError(null);
        try {
            await RpcApi.CreateAgentSkillCommand(TabRpcClient, data);
            this.setEditingSkill(null);
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
        }
    };

    updateSkill = async (data: CommandUpdateAgentSkillData): Promise<void> => {
        this.setError(null);
        try {
            await RpcApi.UpdateAgentSkillCommand(TabRpcClient, data);
            this.setEditingSkill(null);
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
        }
    };

    deleteSkill = async (id: string): Promise<void> => {
        try {
            await RpcApi.DeleteAgentSkillCommand(TabRpcClient, { id });
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
            const entries = await RpcApi.ListAgentHistoryCommand(TabRpcClient, {
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
            const entries = await RpcApi.SearchAgentHistoryCommand(TabRpcClient, {
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
            await RpcApi.ImportAgentFromClawCommand(TabRpcClient, {
                workspace_path: workspacePath,
                agent_name: agentName,
            });
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
        } finally {
            this.setImporting(false);
        }
    };

    // ── Reseed built-in agents ──────────────────────────────────────────────

    reseedAgents = async (): Promise<void> => {
        this.setLoading(true);
        this.setError(null);
        try {
            await RpcApi.ReseedAgentDefinitionsCommand(TabRpcClient);
        } catch (e: any) {
            this.setError(String(e?.message ?? e));
        } finally {
            this.setLoading(false);
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
        this.unsubAgentDefChanged?.();
        this.unsubContentChanged?.();
        this.unsubSkillsChanged?.();
        this.unsubHistoryChanged?.();
    }
}
