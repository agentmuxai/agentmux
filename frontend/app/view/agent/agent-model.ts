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
import { buildBootstrapScript, guessShellType } from "./bootstrap";
import { Logger } from "@/util/logger";
import { stringToBase64 } from "@/util/util";

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
     * Launch an agent in presentation view.
     * For Phase 1, agentId maps to a provider ID (claude/codex/gemini).
     * In Phase 2 this will be a Forge agent UUID.
     */
    launchAgent = async (agentId: string): Promise<void> => {
        const provider = PROVIDERS[agentId];
        if (!provider) {
            Logger.error("agent", "Unknown agent", { agentId });
            return;
        }

        const version = getApi().getAboutModalDetails().version;
        const shellType = guessShellType(getApi().getPlatform());

        Logger.info("agent", `Launching agent ${agentId} (v${version})`, {
            agentId,
            shellType,
            styledArgs: provider.styledArgs,
            outputFormat: provider.styledOutputFormat,
        });

        const oref = WOS.makeORef("block", this.blockId);
        const blockId = this.blockId;
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    "agentId": agentId,
                    "agentOutputFormat": provider.styledOutputFormat,
                    "controller": "shell",
                },
            });

            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: globalStore.get(atoms.staticTabId),
                blockid: blockId,
                forcerestart: true,
            });

            setTimeout(async () => {
                const script = buildBootstrapScript({
                    version,
                    provider,
                    shellType,
                    args: provider.styledArgs,
                });
                const b64data = stringToBase64(script + "\r");
                await RpcApi.ControllerInputCommand(TabRpcClient, {
                    blockid: blockId,
                    inputdata64: b64data,
                });
            }, 500);
        } catch (e: any) {
            Logger.error("agent", "Failed to launch agent", { error: String(e) });
        }
    };

    /**
     * Launch a Forge-managed agent in presentation view.
     * Uses the ForgeAgent's provider to look up CLI config.
     */
    launchForgeAgent = async (agent: ForgeAgent): Promise<void> => {
        const provider = PROVIDERS[agent.provider];
        if (!provider) {
            Logger.error("agent", "Unknown provider in forge agent", { agentId: agent.id, provider: agent.provider });
            return;
        }

        const version = getApi().getAboutModalDetails().version;
        const shellType = guessShellType(getApi().getPlatform());

        Logger.info("agent", `Launching forge agent ${agent.name} (${agent.provider})`, {
            agentId: agent.id,
            provider: agent.provider,
        });

        const oref = WOS.makeORef("block", this.blockId);
        const blockId = this.blockId;
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: agent.id,
                    agentProvider: agent.provider,
                    agentOutputFormat: provider.styledOutputFormat,
                    agentName: agent.name,
                    agentIcon: agent.icon,
                    controller: "shell",
                },
            });

            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: globalStore.get(atoms.staticTabId),
                blockid: blockId,
                forcerestart: true,
            });

            setTimeout(async () => {
                const script = buildBootstrapScript({
                    version,
                    provider,
                    shellType,
                    args: provider.styledArgs,
                });
                const b64data = stringToBase64(script + "\r");
                await RpcApi.ControllerInputCommand(TabRpcClient, {
                    blockid: blockId,
                    inputdata64: b64data,
                });
            }, 500);
        } catch (e: any) {
            Logger.error("agent", "Failed to launch forge agent", { error: String(e) });
        }
    };

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {}
}
