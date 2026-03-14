// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi } from "@/store/global";
import { createEffect, createSignal, Show, type JSX } from "solid-js";

const BackendStatus = (): JSX.Element => {
    const backendStatus = atoms.backendStatusAtom;
    const [popoverOpen, setPopoverOpen] = createSignal(false);
    const [backendInfo, setBackendInfo] = createSignal<{
        pid?: number;
        started_at?: string;
        web_endpoint?: string;
        version: string;
    } | null>(null);
    let popoverRef!: HTMLDivElement;

    const icon = () => {
        switch (backendStatus()) {
            case "running": return "●";
            case "connecting": return "◌";
            case "crashed": return "●";
            default: return null;
        }
    };

    const color = () => {
        switch (backendStatus()) {
            case "running": return "var(--accent-color)";
            case "connecting": return "var(--warning-color)";
            case "crashed": return "var(--error-color)";
            default: return null;
        }
    };

    const label = () => {
        switch (backendStatus()) {
            case "running": return "Backend";
            case "connecting": return "Connecting…";
            case "crashed": return "Backend offline";
            default: return null;
        }
    };

    const iconSpin = () => backendStatus() === "connecting";

    const handleClick = async () => {
        if (popoverOpen()) {
            setPopoverOpen(false);
            return;
        }
        try {
            const info = await getApi().getBackendInfo();
            setBackendInfo(info);
        } catch {
            setBackendInfo(null);
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
        return () => document.removeEventListener("mousedown", handleOutsideClick);
    });

    const formatUptime = (startedAt: string): string => {
        const start = new Date(startedAt).getTime();
        const now = Date.now();
        const secs = Math.floor((now - start) / 1000);
        if (secs < 60) return `${secs}s`;
        const mins = Math.floor(secs / 60);
        if (mins < 60) return `${mins}m`;
        const hrs = Math.floor(mins / 60);
        return `${hrs}h ${mins % 60}m`;
    };

    return (
        <Show when={backendStatus() !== null && icon() !== null}>
            <div style={{ position: "relative" }} ref={popoverRef}>
                <div
                    class="status-bar-item clickable"
                    title="Click for backend details"
                    onClick={handleClick}
                >
                    <span class={`status-icon${iconSpin() ? " status-icon-spin" : ""}`} style={{ color: color() }}>
                        {icon()}
                    </span>
                    <span style={{ color: color() }}>{label()}</span>
                </div>
                <Show when={popoverOpen()}>
                    <div class="status-bar-popover">
                        <div class="status-bar-popover-row">
                            <span class="status-bar-popover-label">Status</span>
                            <span style={{ color: color() }}>{backendStatus()}</span>
                        </div>
                        <Show when={backendInfo()?.pid}>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">PID</span>
                                <span>{backendInfo().pid}</span>
                            </div>
                        </Show>
                        <Show when={backendInfo()?.started_at}>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Uptime</span>
                                <span>{formatUptime(backendInfo().started_at)}</span>
                            </div>
                        </Show>
                        <Show when={backendInfo()?.web_endpoint}>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Endpoint</span>
                                <span class="status-bar-popover-mono">{backendInfo().web_endpoint}</span>
                            </div>
                        </Show>
                        <Show when={backendInfo()?.version}>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Version</span>
                                <span>{backendInfo().version}</span>
                            </div>
                        </Show>
                    </div>
                </Show>
            </div>
        </Show>
    );
};

BackendStatus.displayName = "BackendStatus";

export { BackendStatus };
