// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The status bar's system readouts, the views: the GPU, memory, page file, network and disk I/O readouts of a
// reading from SystemStatsState.ts. (The CPU and Disk buttons, which open the panels, stay in SystemStats.tsx.)

import { t } from "@/app/i18n/i18n";
import { Show, type JSX } from "solid-js";
import { gpuColor } from "./gpu-format";
import { closeGpuPanel, gpuPanelAnchor, toggleGpuPanel } from "./gpu-panel-state";
import { GpuPanel } from "./GpuPanel";
import type { SysStats } from "./SystemStatsState";
import { sharedUiBusy, UI_BUSY_WARN_PCT, uiBusyColor } from "./ui-busy";

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

/**
 * The GPU's use, as a meter beside CPU and Mem, with its separator: the busiest adapter's, from srv's one GPU sampler
 * (`gpu` and `gpu:mem` of the sysinfo event). It is absent while nothing can be read, never 0%. A click opens the
 * GPU panel; the label turns the warning colour at 90% use or 90% memory.
 */
export function GpuReadout(props: Reading): JSX.Element {
    const s = props.s;
    let ref: HTMLButtonElement | undefined;
    const open = () => ref != null && gpuPanelAnchor() === ref;
    return (
        <Show when={s().gpu != null}>
            <span class="stat-separator">|</span>
            <button
                type="button"
                ref={ref}
                class="stat-mono stat-gpu stat-gpu-button"
                style={{ color: gpuColor(s().gpu, s().gpuMem) }}
                data-tip={s().gpuMem != null ? t("gpuMeter.tip", { mem: Math.round(s().gpuMem!) }) : t("gpuMeter.tipNoMem")}
                aria-label={t("gpuMeter.aria")}
                aria-haspopup="dialog"
                aria-expanded={open()}
                onClick={() => ref && toggleGpuPanel(ref)}
            >
                {t("gpuMeter.label")} {Math.round(s().gpu!)}%
            </button>
            <Show when={open()}>
                <GpuPanel anchor={ref} uiBusy={sharedUiBusy()()} onClose={closeGpuPanel} />
            </Show>
        </Show>
    );
}

/** The page's busy share ("UI n%"), with its separator; absent while this window is in the background. */
export function UiBusyReadout(): JSX.Element {
    const busy = sharedUiBusy();
    return (
        <Show when={busy() != null}>
            <span class="stat-separator">|</span>
            <span
                class="stat-mono stat-ui"
                style={{ color: uiBusyColor(busy()!) }}
                data-tip={t("uiBusy.tip", { warn: UI_BUSY_WARN_PCT })}
                aria-label={t("uiBusy.aria")}
            >
                {t("uiBusy.label")} {busy()}%
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
