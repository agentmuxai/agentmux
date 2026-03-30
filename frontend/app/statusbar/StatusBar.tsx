// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi, windowInstanceNumAtom, windowCountAtom, backendStatusAtom } from "@/store/global";
import { Show, type JSX } from "solid-js";
import { BackendStatus } from "./BackendStatus";
import { ConfigStatus } from "./ConfigStatus";
import { ConnectionStatus } from "./ConnectionStatus";
import { HostPopover } from "./HostPopover";
import { SystemStats } from "./SystemStats";
import { UpdateStatus } from "./UpdateStatus";
import "./StatusBar.scss";

const StatusBar = (): JSX.Element => {
    const version = getApi().getAboutModalDetails()?.version ?? "";
    const instanceNum = windowInstanceNumAtom;
    const windowCount = windowCountAtom;

    const handleNewWindow = async () => {
        try {
            await getApi().openNewWindow();
        } catch (error) {
            console.error("[StatusBar] Failed to open new window:", error);
        }
    };

    return (
        <div class="status-bar">
            <div class="status-bar-left">
                <BackendStatus />
                <span class="stat-separator">|</span>
                <SystemStats />
            </div>
            <div class="status-bar-center" />
            <div class="status-bar-right">
                <ConnectionStatus />
                <ConfigStatus />
                <UpdateStatus />
                <HostPopover />
                <Show when={version}>
                    <Show
                        when={backendStatusAtom() !== "crashed"}
                        fallback={
                            <span class="status-version status-version-offline" title="Backend offline">
                                v{version}
                            </span>
                        }
                    >
                        <span
                            class="status-version clickable"
                            onClick={handleNewWindow}
                            title="Open New AgentMux Window"
                        >
                            v{version}
                            <Show when={windowCount() > 1}>
                                <span class="instance-num"> ({instanceNum()})</span>
                            </Show>
                        </span>
                    </Show>
                </Show>
            </div>
        </div>
    );
};

StatusBar.displayName = "StatusBar";

export { StatusBar };
