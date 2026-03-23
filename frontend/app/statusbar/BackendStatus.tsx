// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, backendDeathInfoAtom, getApi } from "@/store/global";
import { waveEventSubscribe } from "@/app/store/wps";
import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

function pad2(n: number): string {
    return n < 10 ? `0${n}` : `${n}`;
}

function formatUptime(secs: number): string {
    const s = secs % 60;
    const m = Math.floor(secs / 60) % 60;
    const h = Math.floor(secs / 3600) % 24;
    const d = Math.floor(secs / 86400);
    if (d > 0) return `${d}:${pad2(h)}:${pad2(m)}:${pad2(s)}`;
    if (h > 0) return `${h}:${pad2(m)}:${pad2(s)}`;
    return `${m}:${pad2(s)}`;
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

    // Drive uptime from sysinfo event timestamp so all windows update in sync.
    // The backend broadcasts sysinfo with a server-side ts (ms epoch); all windows
    // receive the same ts and compute the same integer, eliminating phase drift.
    onMount(() => {
        const unsub = waveEventSubscribe({
            eventType: "sysinfo",
            scope: "local",
            handler: (event) => {
                const ts: number | undefined = (event as WaveEvent)?.data?.ts;
                const start = startedAt();
                if (ts != null && start != null) {
                    setUptimeSecs(Math.floor((ts - start) / 1000));
                }
            },
        });
        onCleanup(() => unsub?.());
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
                        <span
                            class="stat-mono stat-uptime"
                            style={{ "min-width": uptimeSecs() < 3600 ? "5ch" : uptimeSecs() < 86400 ? "8ch" : "12ch" }}
                        >{formatUptime(uptimeSecs())}</span>
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
                        <Show when={backendStatus() === "crashed" && backendDeathInfoAtom() != null}>
                            <div class="status-bar-popover-divider" />
                            <div class="status-bar-popover-row">
                                <span class="status-bar-popover-label">Died at</span>
                                <span>{new Date(backendDeathInfoAtom()!.died_at).toLocaleTimeString()}</span>
                            </div>
                            <Show when={backendDeathInfoAtom()!.uptime_secs != null}>
                                <div class="status-bar-popover-row">
                                    <span class="status-bar-popover-label">Was up</span>
                                    <span>{formatUptime(backendDeathInfoAtom()!.uptime_secs!)}</span>
                                </div>
                            </Show>
                            <Show when={backendDeathInfoAtom()!.code != null}>
                                <div class="status-bar-popover-row">
                                    <span class="status-bar-popover-label">Exit code</span>
                                    <span class="status-bar-popover-mono">{backendDeathInfoAtom()!.code}</span>
                                </div>
                            </Show>
                            <Show when={backendDeathInfoAtom()!.signal != null}>
                                <div class="status-bar-popover-row">
                                    <span class="status-bar-popover-label">Signal</span>
                                    <span class="status-bar-popover-mono">{backendDeathInfoAtom()!.signal}</span>
                                </div>
                            </Show>
                        </Show>
                    </div>
                </Show>
            </div>
        </Show>
    );
};

BackendStatus.displayName = "BackendStatus";

export { BackendStatus };
