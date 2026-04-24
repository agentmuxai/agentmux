// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { waveEventSubscribe } from "@/app/store/wps";
import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

type SysStats = {
    cpu: number;
    gpu: number | null;
    memUsed: number;
    memTotal: number;
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

function cpuColor(pct: number): string {
    if (pct > 95) return "var(--error-color)";
    if (pct > 80) return "var(--warning-color)";
    return "var(--secondary-text-color)";
}

function memColor(used: number, total: number): string {
    if (total <= 0) return "var(--secondary-text-color)";
    if (used / total > 0.9) return "var(--warning-color)";
    return "var(--secondary-text-color)";
}

const SystemStats = (): JSX.Element => {
    const [stats, setStats] = createSignal<SysStats | null>(null);

    onMount(() => {
        const unsub = waveEventSubscribe({
            eventType: "sysinfo",
            scope: "local",
            handler: (event) => {
                const vals = (event as WaveEvent)?.data?.values;
                if (vals == null) return;
                setStats({
                    cpu: vals["cpu"] ?? 0,
                    gpu: vals["gpu"] != null ? vals["gpu"] : null,
                    memUsed: vals["mem:used"] ?? 0,
                    memTotal: vals["mem:total"] ?? 0,
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
                    <span
                        class="stat-mono stat-cpu"
                        style={{ color: cpuColor(s().cpu) }}
                        data-tip-wrap=""
                        data-tip="CPU usage across all cores. Turns amber above 80%, red above 95%."
                        aria-label="CPU usage"
                    >
                        CPU {Math.round(s().cpu)}%
                    </span>
                    <Show when={s().gpu != null}>
                        <span class="stat-separator">|</span>
                        <span
                            class="stat-mono stat-gpu"
                            style={{ color: cpuColor(s().gpu!) }}
                            data-tip-wrap=""
                            data-tip="GPU usage — whichever GPU the system reports as primary."
                            aria-label="GPU usage"
                        >
                            GPU {Math.round(s().gpu!)}%
                        </span>
                    </Show>
                    <span class="stat-separator">|</span>
                    <span
                        class="stat-mono stat-mem"
                        style={{ color: memColor(s().memUsed, s().memTotal) }}
                        data-tip-wrap=""
                        data-tip="Memory in use / total installed. Turns amber over 90%."
                        aria-label="Memory usage"
                    >
                        Mem {formatMemBytes(s().memUsed)}/{formatMemBytes(s().memTotal)}
                    </span>
                    {/* Network indicator stays mounted even at 0/0 so the user
                        can glance at the bar and see "nothing going in or out",
                        instead of wondering whether the widget broke. Zero
                        state is visually muted via CSS. Per
                        SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §4.3. */}
                    <span class="stat-separator">|</span>
                    <span
                        class="stat-mono stat-net"
                        classList={{ "stat-idle": s().netSent === 0 && s().netRecv === 0 }}
                        data-tip-wrap=""
                        data-tip="Network traffic — total upload (↑) and download (↓) across all interfaces. Muted when idle."
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
                        data-tip-wrap=""
                        data-tip="Disk I/O — read rate (R) and write rate (W) across all disks. Muted when idle."
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
