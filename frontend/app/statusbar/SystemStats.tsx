// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { waveEventSubscribe } from "@/app/store/wps";
import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

type SysStats = {
    cpu: number;
    memUsed: number;
    memTotal: number;
};

function formatBytes(gb: number): string {
    if (gb >= 1) return `${gb.toFixed(1)}G`;
    const mb = gb * 1024;
    return `${Math.round(mb)}M`;
}

function cpuColor(cpu: number): string {
    if (cpu > 95) return "var(--error-color)";
    if (cpu > 80) return "var(--warning-color)";
    return "var(--secondary-text-color)";
}

function memColor(used: number, total: number): string {
    if (total <= 0) return "var(--secondary-text-color)";
    const pct = used / total;
    if (pct > 0.9) return "var(--warning-color)";
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
                    memUsed: vals["mem:used"] ?? 0,
                    memTotal: vals["mem:total"] ?? 0,
                });
            },
        });
        onCleanup(() => unsub?.());
    });

    return (
        <Show when={stats()}>
            {(s) => (
                <div class="status-bar-item system-stats">
                    <span class="stat-mono stat-cpu" style={{ color: cpuColor(s().cpu) }}>
                        CPU {Math.round(s().cpu)}%
                    </span>
                    <span class="stat-separator">|</span>
                    <span class="stat-mono stat-mem" style={{ color: memColor(s().memUsed, s().memTotal) }}>
                        Mem {formatBytes(s().memUsed)}/{formatBytes(s().memTotal)}
                    </span>
                </div>
            )}
        </Show>
    );
};

SystemStats.displayName = "SystemStats";

export { SystemStats };
