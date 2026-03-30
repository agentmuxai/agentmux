// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi, lanInstancesAtom } from "@/store/global";
import { invokeCommand } from "@/app/platform/ipc";
import { createEffect, createSignal, For, onCleanup, Show, type JSX } from "solid-js";

type HostInfo = {
    hostname: string;
    os: string;
    localIp: string;
    instanceId: string;
    version: string;
    dataDir: string;
    hostType: string;
    pid: number;
    ports: {
        ipc: string;
        web: string;
        ws: string;
        devtools: string;
    };
};

const HostPopover = (): JSX.Element => {
    const hostname = getApi().getHostName();
    const [popoverOpen, setPopoverOpen] = createSignal(false);
    const [hostInfo, setHostInfo] = createSignal<HostInfo | null>(null);
    let popoverRef!: HTMLDivElement;

    const lanInstances = lanInstancesAtom;
    const lanCount = () => lanInstances().length;

    const handleClick = async () => {
        if (popoverOpen()) {
            setPopoverOpen(false);
            return;
        }
        try {
            const info = await invokeCommand<HostInfo>("get_host_info", {});
            setHostInfo(info);
        } catch {
            // Fallback for Tauri (doesn't have get_host_info yet)
            setHostInfo(null);
        }
        setPopoverOpen(true);
    };

    createEffect(() => {
        if (!popoverOpen()) return;
        const handleOutsideClick = (e: MouseEvent) => {
            if (popoverRef && !popoverRef.contains(e.target as Node)) {
                setPopoverOpen(false);
            }
        };
        document.addEventListener("mousedown", handleOutsideClick);
        onCleanup(() => document.removeEventListener("mousedown", handleOutsideClick));
    });

    return (
        <Show when={hostname && hostname !== "unknown"}>
            <div style={{ position: "relative" }} ref={popoverRef}>
                <div
                    class="status-bar-item clickable"
                    title="Click for host details"
                    onClick={handleClick}
                >
                    <span class="status-hostname">
                        {hostname}
                    </span>
                    <Show when={lanCount() > 0}>
                        <span style={{ color: "var(--accent-color)", "margin-left": "4px" }}>{"◆"}</span>
                    </Show>
                </div>
                <Show when={popoverOpen()}>
                    <div class="status-bar-popover host-popover">
                        {/* Host Identity */}
                        <div class="status-bar-popover-row">
                            <span style={{ "font-weight": "bold", "font-size": "1.05em" }}>
                                {hostInfo()?.hostname ?? hostname}
                            </span>
                        </div>
                        <Show when={hostInfo()}>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">OS</span>
                                <span>{hostInfo()!.os}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">IP</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.localIp}</span>
                            </div>

                            {/* Instance Info */}
                            <div class="status-bar-popover-divider" />
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Instance</span>
                                <span>{hostInfo()!.instanceId}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Host</span>
                                <span>{hostInfo()!.hostType}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">PID</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.pid}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Data</span>
                                <span class="status-bar-popover-mono" style={{ "font-size": "0.85em", "max-width": "220px", "overflow": "hidden", "text-overflow": "ellipsis" }}>
                                    {hostInfo()!.dataDir}
                                </span>
                            </div>

                            {/* Network */}
                            <div class="status-bar-popover-divider" />
                            <Show when={lanCount() > 0}>
                                <div class="status-bar-popover-row">
                                    <span style={{ color: "var(--accent-color)" }}>{"◆"}</span>
                                    <span>{lanCount()} instance{lanCount() !== 1 ? "s" : ""} on LAN</span>
                                </div>
                                <For each={lanInstances()}>
                                    {(inst: LanInstance) => (
                                        <div class="status-bar-popover-row" style={{ "padding-left": "12px" }}>
                                            <span style={{ opacity: "0.7" }}>{inst.hostname || inst.instance_id}</span>
                                            <span class="status-bar-popover-mono" style={{ opacity: "0.5" }}>v{inst.version}</span>
                                        </div>
                                    )}
                                </For>
                                <div class="status-bar-popover-divider" />
                            </Show>
                            <Show when={lanCount() === 0}>
                                <div class="status-bar-popover-row" style={{ opacity: "0.5" }}>
                                    <span>No LAN peers found</span>
                                </div>
                                <div class="status-bar-popover-row" style={{ opacity: "0.4", "font-size": "0.85em" }}>
                                    <span>Enable via "network:lan_discovery": true</span>
                                </div>
                                <div class="status-bar-popover-divider" />
                            </Show>

                            {/* Ports */}
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">IPC</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.ports.ipc}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Backend</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.ports.web}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">WS</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.ports.ws}</span>
                            </div>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">DevTools</span>
                                <span class="status-bar-popover-mono">{hostInfo()!.ports.devtools}</span>
                            </div>
                        </Show>
                    </div>
                </Show>
            </div>
        </Show>
    );
};

HostPopover.displayName = "HostPopover";

export { HostPopover };
