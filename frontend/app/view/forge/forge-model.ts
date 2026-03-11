// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { waveEventSubscribe } from "@/app/store/wps";
import { atom, PrimitiveAtom } from "jotai";

export type ForgeView = "list" | "create" | "edit";

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

    private unsubForgeChanged: (() => void) | null = null;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.loadAgents();
        this.unsubForgeChanged = waveEventSubscribe({
            eventType: "forgeagents:changed",
            handler: () => this.loadAgents(),
        });
    }

    loadAgents = async (): Promise<void> => {
        try {
            const agents = await RpcApi.ListForgeAgentsCommand(TabRpcClient);
            // Access globalStore via import to avoid circular deps
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

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {
        this.unsubForgeChanged?.();
    }
}
