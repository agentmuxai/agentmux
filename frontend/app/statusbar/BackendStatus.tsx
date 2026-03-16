// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi } from "@/store/global";
import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

function formatUptime(secs: number): string {
    if (secs < 60) return `${secs}s`;
    const mins = Math.floor(secs / 60);
    if (mins < 60) return `${mins}m ${secs % 60}s`;
    const hrs = Math.floor(mins / 60);
    if (hrs < 24) return `${hrs}h ${mins % 60}m`;
    const days = Math.floor(hrs / 24);
    if (days < 7) return `${days}d ${hrs % 24}h`;
    const weeks = Math.floor(days / 7);
    if (days < 30) return `${weeks}w ${days % 7}d`;
    const months = Math.floor(days / 30);
    if (months < 12) return `${months}mo ${days % 30}d`;
    const years = Math.floor(months / 12);
    return `${years}yr ${months % 12}mo`;
}

const BackendStatus = (): JSX.Element => {
    const backendStatus = atoms.backendStatusAtom;
    const [popoverOpen, setPopoverOpen] = createSignal(false);
    const [startedAt, setStartedAt] = createSignal<number | null>(null);
    const [uptimeSecs, setUptimeSecs] = createSignal(0);
    const [backendInfo, setBackendInfo] = createSignal<{
        pid?: number;
        started_at?: string;
        web_endpoint?: string;
        version: string;
    } | null>(null);
    let popoverRef!: HTMLDivElement;

    // Fetch started_at when backend becomes running
    createEffect(() => {
        const status = backendStatus();
        if (status === "running" && startedAt() == null) {
            getApi().getBackendInfo().then((info) => {
                setBackendInfo(info);
                if (info?.started_at) {
                    setStartedAt(new Date(info.started_at).getTime());
                }
            }).catch(() => {});
        }
    });

    // Tick uptime every second
    onMount(() => {
        const iv = setInterval(() => {
            const start = startedAt();
            if (start != null) {
                setUptimeSecs(Math.floor((Date.now() - start) / 1000));
            }
        }, 1000);
        onCleanup(() => clearInterval(iv));
    });

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
                    <Show when={backendStatus() === "running" && startedAt() != null}>
                        <span class="stat-mono">{formatUptime(uptimeSecs())}</span>
                    </Show>
                    <Show when={backendStatus() === "connecting"}>
                        <span style={{ color: color() }}>Connecting…</span>
                    </Show>
                    <Show when={backendStatus() === "crashed"}>
                        <span style={{ color: color() }}>Offline</span>
                    </Show>
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
                        <Show when={startedAt() != null}>
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Uptime</span>
                                <span>{formatUptime(uptimeSecs())}</span>
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
