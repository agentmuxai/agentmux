// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi, windowCountAtom, backendStatusAtom, isDev } from "@/store/global";
import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { BackendStatus } from "./BackendStatus";
import { ConfigStatus } from "./ConfigStatus";
import { GpuStatus } from "./GpuStatus";
import { HostPopover } from "./HostPopover";
import { InstancePanel } from "./InstancePanel";
import { StatusBarTip } from "./StatusBarTip";
import { SystemStats } from "./SystemStats";
import { TokenUsageIndicator } from "./TokenUsageIndicator";
import { UpdateStatus } from "./UpdateStatus";
import "./StatusBar.scss";

const StatusBar = (): JSX.Element => {
    const version = getApi().getAboutModalDetails()?.version ?? "";
    const windowCount = windowCountAtom;

    // The version chip: the button, or the offline span while the backend is
    // down. The instance panel anchors to whichever is showing ("Open
    // Maintenance" can open it while the backend is down).
    let versionRef: HTMLElement | undefined;
    const [panelOpen, setPanelOpen] = createSignal(false);

    const handleVersionClick = () => setPanelOpen(!panelOpen());

    // BackendStatus dot can request the version panel to open (e.g. "Open Maintenance ↗").
    onMount(() => {
        const handler = () => setPanelOpen(true);
        window.addEventListener("agentmux:open-version-panel", handler);
        onCleanup(() => window.removeEventListener("agentmux:open-version-panel", handler));
    });

    return (
        <div class="status-bar">
            <div class="status-bar-left">
                <BackendStatus />
                <span class="stat-separator">|</span>
                <SystemStats />
                <span class="stat-separator">|</span>
                <GpuStatus />
            </div>
            <div class="status-bar-center" />
            <div class="status-bar-right">
                <TokenUsageIndicator />
                <ConfigStatus />
                <UpdateStatus />
                <HostPopover />
                <Show when={version}>
                    <Show
                        when={backendStatusAtom() !== "crashed"}
                        fallback={
                            <span
                                ref={(el) => { versionRef = el; }}
                                class="status-version status-version-offline"
                                data-tip="Backend offline"
                                aria-label="Backend offline"
                            >
                                v{version}
                                <Show when={isDev()}>
                                    <span class="status-version-dev">DEV</span>
                                </Show>
                            </span>
                        }
                    >
                        <button
                            ref={(el) => { versionRef = el; }}
                            type="button"
                            class="status-version clickable"
                            onClick={handleVersionClick}
                            data-tip="Click for instance panel"
                            aria-label="AgentMux version — open instance panel"
                            aria-haspopup="dialog"
                            aria-expanded={panelOpen()}
                        >
                            v{version}
                            <Show when={isDev()}>
                                <span class="status-version-dev">DEV</span>
                            </Show>
                            <Show when={windowCount() > 1}>
                                <span class="instance-num"> ({windowCount()})</span>
                            </Show>
                        </button>
                    </Show>
                </Show>
            </div>
            <Show when={panelOpen()}>
                <InstancePanel anchor={versionRef} onClose={() => setPanelOpen(false)} />
            </Show>
            <StatusBarTip />
        </div>
    );
};

StatusBar.displayName = "StatusBar";

export { StatusBar };
