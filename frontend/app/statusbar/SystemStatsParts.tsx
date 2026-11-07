// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The status bar's system readouts, the views: the GPU, memory, page file, network and disk I/O readouts of a
// reading from SystemStatsState.ts. (The CPU and Disk buttons, which open the panels, stay in SystemStats.tsx.)

import { Show, type JSX } from "solid-js";
import { cpuColor } from "./cpu-color";
import type { SysStats } from "./SystemStatsState";

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
    if (used / total > 0.9) return "var(--warning-text-color)";
    return "var(--secondary-text-color)";
}

function commitColor(used: number, total: number): string {
    if (total <= 0) return "var(--secondary-text-color)";
    const ratio = used / total;
    if (ratio > 0.95) return "var(--error-text-color)";
    if (ratio > 0.85) return "var(--warning-text-color)";
    return "var(--secondary-text-color)";
}

type Reading = { s: () => SysStats };

/** The GPU usage, with its separator, while there is a GPU reading. */
export function GpuReadout(props: Reading): JSX.Element {
    const s = props.s;
    return (
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
    );
}

/** Memory used and total, then, while there is a commit total, the page file's. */
export function MemReadouts(props: Reading): JSX.Element {
    const s = props.s;
    return (
        <>
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
                    data-tip="Commit charge used and total (RAM + page file budget)."
                    aria-label="Commit charge"
                >
                    PF {formatMemBytes(s().commitUsed)}/{formatMemBytes(s().commitTotal)}
                </span>
            </Show>
        </>
    );
}

/** The network and disk I/O rates. */
export function RateReadouts(props: Reading): JSX.Element {
    const s = props.s;
    return (
        <>
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
                <span class="stat-disk-arrow">↑</span>
                {formatRate(s().netSent)} <span class="stat-disk-arrow">↓</span>
                {formatRate(s().netRecv)}
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
                <span class="stat-disk-arrow">R</span>
                {formatRate(s().diskRead)} <span class="stat-disk-arrow">W</span>
                {formatRate(s().diskWrite)}
            </span>
        </>
    );
}
