// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { useWidthFit } from "@/app/element/width-fit";
import { chromeZoomAtom } from "@/app/store/chrome-zoom";
import { getApi, windowCountAtom, backendStatusAtom, isDev } from "@/store/global";
import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { BackendStatus } from "./BackendStatus";
import { ConfigStatus } from "./ConfigStatus";
import { ConnectivityIndicator } from "./ConnectivityIndicator";
import { BrowserPauseAllStatus } from "./BrowserPauseAllStatus";
import { GpuStatus } from "./GpuStatus";
import { HostPopover } from "./HostPopover";
import { InstancePanel } from "./InstancePanel";
import { AgentCountsStatus } from "./AgentCountsStatus";
import { VaultRequestsStatus } from "./VaultRequestsStatus";
import { MultiInputStatus } from "./MultiInputStatus";
import { MuxBusIndicator } from "./MuxBusIndicator";
import { registerStatusBarItem, statusBarItems, type StatusBarItem } from "./status-bar-registry";
import { StatusBarTip } from "./StatusBarTip";
import { SystemStats } from "./SystemStats";
import { HistoryUsageIndicator } from "./HistoryUsageIndicator";
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

/** Which fit tier hides each of AgentMux's items in a narrow bar (the rest stay: the backend dot, vault
 *  requests, update and version notices). Higher tiers hide more; before it goes, system-stats sheds its
 *  readouts in StatusBar.scss. */
const DROP_AT: Record<string, 1 | 2 | 3 | 4 | undefined> = {
    gpu: 2,
    "history-usage": 2,
    "token-usage": 2,
    "agent-counts": 3,
    "browser-pause-all": 3,
    "multi-input": 3,
    config: 3,
    host: 3,
    connectivity: 4,
    muxbus: 4,
    "system-stats": 4,
};

/** What a narrow bar sheds, tier by tier, until it fits on one line. */
const FIT_TIERS = ["1", "2", "3", "4"] as const;

/** A widget's item goes before AgentMux's own readouts do. */
const WIDGET_DROP_AT = 2;

/** AgentMux's own items, in the status bar registry. Widgets add theirs
 *  there too (`widget-contributions.tsx`). */
function registerBuiltinStatusBarItems(): void {
    if (builtinsRegistered) return;
    builtinsRegistered = true;
    const builtins: [string, "left" | "right", () => JSX.Element][] = [
        ["backend", "left", () => <BackendStatus />],
        ["system-stats", "left", () => <SystemStats />],
        ["gpu", "left", () => <GpuStatus />],
        ["agent-counts", "right", () => <AgentCountsStatus />],
        ["vault-requests", "right", () => <VaultRequestsStatus />],
        ["browser-pause-all", "right", () => <BrowserPauseAllStatus />],
        ["multi-input", "right", () => <MultiInputStatus />],
        ["history-usage", "right", () => <HistoryUsageIndicator />],
        ["token-usage", "right", () => <TokenUsageIndicator />],
        ["config", "right", () => <ConfigStatus />],
        ["connectivity", "right", () => <ConnectivityIndicator />],
        ["muxbus", "right", () => <MuxBusIndicator />],
        ["version-skew", "right", () => <VersionSkewStatus />],
        ["update", "right", () => <UpdateStatus />],
        ["host", "right", () => <HostPopover />],
        ["version", "right", () => <VersionChip />],
    ];
    const next = { left: 0, right: 0 };
    for (const [id, side, render] of builtins) {
        next[side] += 100;
        registerStatusBarItem({ id: `agentmux:${id}`, side, order: next[side], render, dropAt: DROP_AT[id] });
    }
}

const StatusBar = (): JSX.Element => {
    registerBuiltinStatusBarItems();
    let barRef: HTMLDivElement | undefined;
    useWidthFit(() => barRef, FIT_TIERS, (el) => [...el.querySelectorAll(".status-bar-left, .status-bar-right")], chromeZoomAtom);
    // The separator goes with its item, so a hidden item leaves none behind.
    const slot = (item: StatusBarItem, separated: boolean) => (
        <span class="status-bar-slot" data-drop-at={item.dropAt ?? (item.id.startsWith("agentmux:") ? undefined : WIDGET_DROP_AT)}>
            <Show when={separated}>
                <span class="stat-separator">|</span>
            </Show>
            {item.render()}
        </span>
    );
    return (
        <div class="status-bar" data-toast-avoid ref={barRef}>
            <div class="status-bar-left">
                <For each={statusBarItems("left")}>{(item, i) => slot(item, i() > 0)}</For>
            </div>
            <div class="status-bar-center" />
            <div class="status-bar-right">
                <For each={statusBarItems("right")}>{(item) => slot(item, false)}</For>
            </div>
            <StatusBarTip />
        </div>
    );
};

StatusBar.displayName = "StatusBar";

export { StatusBar };
