// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, Show, type JSX } from "solid-js";
import { BackendStatus } from "./BackendStatus";
import { ConfigStatus } from "./ConfigStatus";
import { GpuStatus } from "./GpuStatus";
import { HostPopover } from "./HostPopover";
import { MuxBusIndicator } from "./MuxBusIndicator";
import { registerStatusBarItem, statusBarItems } from "./status-bar-registry";
import { StatusBarTip } from "./StatusBarTip";
import { SystemStats } from "./SystemStats";
import { TokenUsageIndicator } from "./TokenUsageIndicator";
import { UpdateStatus } from "./UpdateStatus";
import { VersionChip } from "./VersionChip";
import { VersionSkewStatus } from "./VersionSkewStatus";
import "./StatusBar.scss";

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
