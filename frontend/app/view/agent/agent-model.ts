// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, globalStore, WOS } from "@/app/store/global";
import { atom, Atom, PrimitiveAtom } from "jotai";
import React from "react";
import { AgentViewWrapper } from "./agent-view";
import { PROVIDERS } from "./providers";
import { Logger } from "@/util/logger";

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
     * Switches the block to a terminal view running the provider's CLI.
     * This unmounts the AgentViewModel and creates a TermViewModel.
     */
    connectWithProvider = async (providerId: string, cliPath: string): Promise<void> => {
        const provider = PROVIDERS[providerId];
        if (!provider) {
            Logger.error("agent", "Unknown provider", { providerId });
            return;
        }

        Logger.info("agent", `Starting ${provider.id} — switching to terminal view`, {
            provider: provider.id,
            cliPath,
            args: provider.defaultArgs,
        });

        const oref = WOS.makeORef("block", this.blockId);
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    "view": "term",
                    "controller": "cmd",
                    "cmd": cliPath,
                    "cmd:args": provider.defaultArgs,
                    "cmd:interactive": false,
                    "cmd:runonstart": true,
                },
            });
            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: globalStore.get(atoms.staticTabId),
                blockid: this.blockId,
                forcerestart: true,
            });
        } catch (e: any) {
            Logger.error("agent", "Failed to start session", { error: String(e) });
        }
    };

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {}
}
