// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, getApi, globalStore, WOS } from "@/app/store/global";
import { atom, Atom, PrimitiveAtom } from "jotai";
import React from "react";
import { AgentViewWrapper } from "./agent-view";
import { PROVIDERS } from "./providers";

export type ProviderStatus = "idle" | "detecting" | "installing" | "launching" | "error";

export class AgentViewModel implements ViewModel {
    viewType = "agent";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: Atom<Block>;

    viewIcon: Atom<string>;
    viewName: Atom<string>;
    viewText: Atom<string | HeaderElem[]>;
    viewComponent: ViewComponent;
    noPadding = atom(true);

    // Observable status for the agent view UI
    providerStatus: PrimitiveAtom<ProviderStatus> = atom<ProviderStatus>("idle");
    statusMessage: PrimitiveAtom<string> = atom("");

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
        this.viewComponent = AgentViewWrapper as any;

        this.viewIcon = atom("sparkles");
        this.viewName = atom("Agent");
        this.viewText = atom<string | HeaderElem[]>([]);
    }

    /**
     * Called when user clicks a provider button.
     * Detects if CLI is installed, installs if needed, then launches terminal.
     */
    connectWithProvider = async (providerId: string): Promise<void> => {
        const provider = PROVIDERS[providerId];
        if (!provider) {
            console.error("[agent] Unknown provider:", providerId);
            return;
        }

        const api = getApi();

        // 1. Detect CLI
        globalStore.set(this.providerStatus, "detecting");
        globalStore.set(this.statusMessage, `Detecting ${provider.displayName}...`);
        console.log(`[agent] Detecting ${provider.id} CLI...`);

        let cliPath: string | null = null;
        try {
            cliPath = await api.getCliPath(provider.id);
        } catch (e: any) {
            console.error("[agent] CLI detection failed:", e);
        }

        // 2. Install if not found
        if (!cliPath) {
            globalStore.set(this.providerStatus, "installing");
            globalStore.set(this.statusMessage, `Installing ${provider.displayName}...`);
            console.log(`[agent] ${provider.id} not found, installing...`);

            try {
                const result = await api.installCli(provider.id);
                cliPath = result.cli_path;
                console.log(`[agent] Installed ${provider.id} at ${cliPath}`);
            } catch (e: any) {
                console.error("[agent] CLI install failed:", e);
                globalStore.set(this.providerStatus, "error");
                globalStore.set(this.statusMessage, `Failed to install ${provider.displayName}: ${e.message || e}`);
                return;
            }
        }

        // 3. Launch terminal with resolved path
        globalStore.set(this.providerStatus, "launching");
        globalStore.set(this.statusMessage, `Launching ${provider.displayName}...`);
        console.log(`[agent] Starting ${provider.id} from ${cliPath}`);

        const oref = WOS.makeORef("block", this.blockId);
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    "view": "term",
                    "controller": "cmd",
                    "cmd": cliPath,
                    "cmd:args": provider.defaultArgs,
                    "cmd:interactive": true,
                    "cmd:runonstart": true,
                },
            });
            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: globalStore.get(atoms.staticTabId),
                blockid: this.blockId,
                forcerestart: true,
            });
        } catch (e: any) {
            console.error("[agent] Failed to start session:", e);
            globalStore.set(this.providerStatus, "error");
            globalStore.set(this.statusMessage, `Failed to launch ${provider.displayName}`);
        }
    };

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {}
}
