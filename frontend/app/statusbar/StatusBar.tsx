// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi, windowCountAtom, backendStatusAtom, isDev } from "@/store/global";
import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { BackendStatus } from "./BackendStatus";
import { ConfigStatus } from "./ConfigStatus";
import { GpuStatus } from "./GpuStatus";
import { HostPopover } from "./HostPopover";
import { InstancePanel } from "./InstancePanel";
import { MuxBusIndicator } from "./MuxBusIndicator";
import { registerStatusBarItem, statusBarItems } from "./status-bar-registry";
import { StatusBarTip } from "./StatusBarTip";
import { SystemStats } from "./SystemStats";
import { TokenUsageIndicator } from "./TokenUsageIndicator";
import { UpdateStatus } from "./UpdateStatus";
import { VersionSkewStatus } from "./VersionSkewStatus";
import "./StatusBar.scss";

/** The version chip at the right end of the status bar, and the instance
 *  panel it opens. */
const VersionChip = (): JSX.Element => {
    const version = getApi().getAboutModalDetails()?.version ?? "";
    const windowCount = windowCountAtom;

    // The chip is the button, or the offline span while the backend is down.
    // The instance panel anchors to whichever is showing ("Open Maintenance"
    // can open it while the backend is down).
    let versionRef: HTMLElement | undefined;
    const [panelOpen, setPanelOpen] = createSignal(false);

    // BackendStatus dot can request the version panel to open (e.g. "Open Maintenance ↗").
    onMount(() => {
        const handler = () => setPanelOpen(true);
        window.addEventListener("agentmux:open-version-panel", handler);
        onCleanup(() => window.removeEventListener("agentmux:open-version-panel", handler));
    });

    return (
        <>
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
                        onClick={() => setPanelOpen(!panelOpen())}
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
            <Show when={panelOpen()}>
                <InstancePanel anchor={versionRef} onClose={() => setPanelOpen(false)} />
            </Show>
        </>
    );
};

let builtinsRegistered = false;

/** AgentMux's own items, in the status bar registry. Widgets add theirs
 *  there too (`widget-contributions.tsx`). */
export function registerBuiltinStatusBarItems(): void {
    if (builtinsRegistered) return;
    builtinsRegistered = true;
    const builtins: [string, "left" | "right", () => JSX.Element][] = [
        ["backend", "left", () => <BackendStatus />],
        ["system-stats", "left", () => <SystemStats />],
        ["gpu", "left", () => <GpuStatus />],
        ["token-usage", "right", () => <TokenUsageIndicator />],
        ["config", "right", () => <ConfigStatus />],
        ["muxbus", "right", () => <MuxBusIndicator />],
        ["version-skew", "right", () => <VersionSkewStatus />],
        ["update", "right", () => <UpdateStatus />],
        ["host", "right", () => <HostPopover />],
        ["version", "right", () => <VersionChip />],
    ];
    const next = { left: 0, right: 0 };
    for (const [id, side, render] of builtins) {
        next[side] += 100;
        registerStatusBarItem({ id: `agentmux:${id}`, side, order: next[side], render });
    }
}

const StatusBar = (): JSX.Element => {
    registerBuiltinStatusBarItems();
    return (
        <div class="status-bar">
            <div class="status-bar-left">
                <For each={statusBarItems("left")}>
                    {(item, i) => (
                        <>
                            <Show when={i() > 0}>
                                <span class="stat-separator">|</span>
                            </Show>
                            {item.render()}
                        </>
                    )}
                </For>
            </div>
            <div class="status-bar-center" />
            <div class="status-bar-right">
                <For each={statusBarItems("right")}>{(item) => item.render()}</For>
            </div>
            <StatusBarTip />
        </div>
    );
};

StatusBar.displayName = "StatusBar";

export { StatusBar };
