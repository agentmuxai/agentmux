// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, getApi, globalStore, WOS } from "@/app/store/global";
import { createSignal } from "solid-js";
import { SignalAtom } from "@/util/util";
import { AgentViewWrapper } from "./agent-view";
import { PROVIDERS } from "./providers";
import { buildBootstrapScript, guessShellType } from "./bootstrap";
import { Logger } from "@/util/logger";
import { stringToBase64 } from "@/util/util";

export class AgentViewModel implements ViewModel {
    viewType = "agent";
    blockId: string;
    nodeModel: BlockNodeModel;
    blockAtom: SignalAtom<Block>;

    viewIcon: () => string;
    viewName: () => string;
    viewText: () => string | HeaderElem[];
    viewComponent: ViewComponent;
    noPadding: () => boolean;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
        this.viewComponent = AgentViewWrapper as any;

        this.viewIcon = () => "sparkles";
        this.viewName = () => "Agent";
        this.viewText = () => [] as HeaderElem[];
        this.noPadding = () => true;
    }

    /**
     * Called when user clicks a provider button (raw mode).
     */
    connectWithProvider = async (providerId: string, _cliPath: string): Promise<void> => {
        const provider = PROVIDERS[providerId];
        if (!provider) {
            Logger.error("agent", "Unknown provider", { providerId });
            return;
        }

        const version = getApi().getAboutModalDetails().version;
        const shellType = guessShellType(getApi().getPlatform());

        Logger.info("agent", `Starting ${provider.id} — isolated CLI (v${version})`, {
            provider: provider.id,
            shellType,
            args: provider.defaultArgs,
        });

        const oref = WOS.makeORef("block", this.blockId);
        const blockId = this.blockId;
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    "view": "term",
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
                    args: provider.defaultArgs,
                });
                const b64data = stringToBase64(script + "\r");
                await RpcApi.ControllerInputCommand(TabRpcClient, {
                    blockid: blockId,
                    inputdata64: b64data,
                });
            }, 500);
        } catch (e: any) {
            Logger.error("agent", "Failed to start session", { error: String(e) });
        }
    };

    /**
     * Called when user clicks a styled provider button.
     */
    connectStyled = async (providerId: string, _cliPath: string): Promise<void> => {
        const provider = PROVIDERS[providerId];
        if (!provider) {
            Logger.error("agent", "Unknown provider", { providerId });
            return;
        }

        const version = getApi().getAboutModalDetails().version;
        const shellType = guessShellType(getApi().getPlatform());

        Logger.info("agent", `Starting ${provider.id} in styled mode (v${version})`, {
            provider: provider.id,
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
                    "agentMode": "styled",
                    "agentProvider": provider.id,
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
            Logger.error("agent", "Failed to start styled session", { error: String(e) });
        }
    };

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {}
}
