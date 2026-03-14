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

export const CONTENT_TAB_LABELS: Record<ContentTabId, string> = {
    soul: "Soul",
    agentmd: "Instructions",
    mcp: "MCP",
    env: "Env",
};

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
    contentLoadingAtom: PrimitiveAtom<boolean> = atom(false);
    contentSavingAtom: PrimitiveAtom<boolean> = atom(false);

    private unsubForgeChanged: (() => void) | null = null;
    private unsubContentChanged: (() => void) | null = null;

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
        globalStore.set(this.contentAtom, {});
        globalStore.set(this.viewAtom, "detail");
        await this.loadContent(agent.id);
    };

    closeDetail = async (): Promise<void> => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(this.detailAgentAtom, null);
        globalStore.set(this.contentAtom, {});
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
    }
}
