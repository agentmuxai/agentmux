// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { CpuCoresPopover } from "./CpuCoresPopover";
import { cpuColor } from "./cpu-color";

type SysStats = {
    cpu: number;
    gpu: number | null;
    memUsed: number;
    memTotal: number;
    commitUsed: number;
    commitTotal: number;
    diskRead: number;
    diskWrite: number;
    netSent: number;
    netRecv: number;
};

function formatMemBytes(gb: number): string {
    if (gb >= 1) return `${gb.toFixed(1)}G`;
    const mb = gb * 1024;
    return `${Math.round(mb)}M`;
}

function formatRate(mbps: number): string {
    if (mbps >= 1000) return `${(mbps / 1024).toFixed(1)}G`;
    if (mbps >= 1) return `${mbps.toFixed(1)}M`;
    const kbps = mbps * 1024;
    if (kbps >= 1) return `${Math.round(kbps)}K`;
    return "0K";
}

function memColor(used: number, total: number): string {
    if (total <= 0) return "var(--secondary-text-color)";
    if (used / total > 0.9) return "var(--warning-color)";
    return "var(--secondary-text-color)";
}

function commitColor(used: number, total: number): string {
    if (total <= 0) return "var(--secondary-text-color)";
    const ratio = used / total;
    if (ratio > 0.95) return "var(--error-color)";
    if (ratio > 0.85) return "var(--warning-color)";
    return "var(--secondary-text-color)";
}

const SystemStats = (): JSX.Element => {
    const [stats, setStats] = createSignal<SysStats | null>(null);

    // Per-core CPU panel — opened by clicking the CPU readout. Mirrors the
    // TokenUsageIndicator → TokenBreakdownPopover interaction.
    const [cpuPanelOpen, setCpuPanelOpen] = createSignal(false);
    const [cpuAnchorRect, setCpuAnchorRect] = createSignal<DOMRect | null>(null);
    let cpuButtonRef: HTMLButtonElement | undefined;
    let cpuPopoverRef: HTMLDivElement | undefined;

    const toggleCpuPanel = () => {
        if (cpuPanelOpen()) {
            setCpuPanelOpen(false);
            return;
        }
        if (cpuButtonRef) setCpuAnchorRect(cpuButtonRef.getBoundingClientRect());
        setCpuPanelOpen(true);
    };

    // Close on outside click (ignoring the button + popover) and on Esc.
    createEffect(() => {
        if (!cpuPanelOpen()) return;
        const onDown = (e: MouseEvent) => {
            const t = e.target as Node;
            if (cpuButtonRef?.contains(t) || cpuPopoverRef?.contains(t)) return;
            setCpuPanelOpen(false);
        };
        const onKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                e.stopPropagation();
                setCpuPanelOpen(false);
            }
        };
        document.addEventListener("mousedown", onDown, true);
        window.addEventListener("keydown", onKey, true);
        onCleanup(() => {
            document.removeEventListener("mousedown", onDown, true);
            window.removeEventListener("keydown", onKey, true);
        });
    });

    onMount(() => {
        const unsub = waveEventSubscribe({
            eventType: WpsEvent.SysInfo,
            scope: "local",
            handler: (event) => {
                const vals = (event as WaveEvent)?.data?.values;
                if (vals == null) return;
                setStats({
                    cpu: vals["cpu"] ?? 0,
                    gpu: vals["gpu"] != null ? vals["gpu"] : null,
                    memUsed: vals["mem:used"] ?? 0,
                    memTotal: vals["mem:total"] ?? 0,
                    commitUsed: vals["mem:commit:used"] ?? 0,
                    commitTotal: vals["mem:commit:total"] ?? 0,
                    diskRead: vals["disk:read"] ?? 0,
                    diskWrite: vals["disk:write"] ?? 0,
                    netSent: vals["net:bytessent"] ?? 0,
                    netRecv: vals["net:bytesrecv"] ?? 0,
                });
            },
        });
        onCleanup(() => unsub?.());
    });

    return (
        <Show when={stats()}>
            {(s) => (
                <div class="status-bar-item system-stats">
                    <button
                        type="button"
                        ref={cpuButtonRef}
                        class="stat-mono stat-cpu stat-cpu-button"
                        style={{ color: cpuColor(s().cpu) }}
                        onClick={toggleCpuPanel}
                        data-tip="Per-core CPU usage"
                        aria-label="CPU usage, click for per-core breakdown"
                        aria-haspopup="dialog"
                        aria-expanded={cpuPanelOpen()}
                    >
                        CPU {Math.round(s().cpu)}%
                    </button>
                    <Show when={cpuPanelOpen()}>
                        <Portal>
                            <CpuCoresPopover
                                anchorRect={cpuAnchorRect()}
                                ref={(el) => { cpuPopoverRef = el; }}
                            />
                        </Portal>
                    </Show>
                    <Show when={s().gpu != null}>
                        <span class="stat-separator">|</span>
                        <span
                            class="stat-mono stat-gpu"
                            style={{ color: cpuColor(s().gpu!) }}
                            data-tip="GPU usage"
                            aria-label="GPU usage"
                        >
                            GPU {Math.round(s().gpu!)}%
                        </span>
                    </Show>
                    <span class="stat-separator">|</span>
                    <span
                        class="stat-mono stat-mem"
                        style={{ color: memColor(s().memUsed, s().memTotal) }}
                        data-tip="Memory used and total"
                        aria-label="Memory usage"
                    >
                        Mem {formatMemBytes(s().memUsed)}/{formatMemBytes(s().memTotal)}
                    </span>
                    <Show when={s().commitTotal > 0}>
                        <span class="stat-separator">|</span>
                        <span
                            class="stat-mono stat-commit"
                            style={{ color: commitColor(s().commitUsed, s().commitTotal) }}
                            data-tip="Commit charge used and total (RAM + page file budget). High commit causes OOM kills."
                            aria-label="Commit charge"
                        >
                            PF {formatMemBytes(s().commitUsed)}/{formatMemBytes(s().commitTotal)}
                        </span>
                    </Show>
                    {/* Network indicator stays mounted even at 0/0 so the user
                        can glance at the bar and see "nothing going in or out",
                        instead of wondering whether the widget broke. Zero
                        state is visually muted via CSS. Per
                        SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §4.3. */}
                    <span class="stat-separator">|</span>
                    <span
                        class="stat-mono stat-net"
                        classList={{ "stat-idle": s().netSent === 0 && s().netRecv === 0 }}
                        data-tip="Network upload and download"
                        aria-label="Network traffic"
                    >
                        <span class="stat-disk-arrow">↑</span>{formatRate(s().netSent)}{" "}
                        <span class="stat-disk-arrow">↓</span>{formatRate(s().netRecv)}
                    </span>
                    {/* Disk I/O stays mounted even at 0/0 so the bar's layout
                        is stable — matches the network widget's always-visible
                        treatment (§4.3 of SPEC_STATUSBAR_TOKEN_USAGE).
                        Windows note: sysinfo currently reports disk read/write
                        as zero; readout will show muted `R0K W0K` until that's
                        addressed. Preferable to a missing widget. */}
                    <span class="stat-separator">|</span>
                    <span
                        class="stat-mono stat-disk"
                        classList={{ "stat-idle": s().diskRead === 0 && s().diskWrite === 0 }}
                        data-tip="Disk read and write"
                        aria-label="Disk I/O"
                    >
                        <span class="stat-disk-arrow">R</span>{formatRate(s().diskRead)}{" "}
                        <span class="stat-disk-arrow">W</span>{formatRate(s().diskWrite)}
                    </span>
                </div>
            )}
        </Show>
    );
};

SystemStats.displayName = "SystemStats";

export { SystemStats };
