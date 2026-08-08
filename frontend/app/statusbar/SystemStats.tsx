// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { CpuCoresPopover } from "./CpuCoresPopover";
import { DiskVolumesPopover } from "./DiskVolumesPopover";
import { cpuColor } from "./cpu-color";
import { diskTooltip, parseDiskVolumes, type DiskVolume } from "./disk-volumes";

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
    pagefileVolumeFreeGb: number | null;
    pagefileVolumeFreePct: number | null;
    pagefileSystemManaged: boolean;
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

// SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 §5.2 P0 — the commit gauge above
// only sees the SYMPTOM (commit near limit); it's blind to the CAUSE this
// spec found: a system-managed page file wants to auto-grow toward
// min(3×RAM, ⅛ volume) but silently can't if the volume it lives on is low
// on free space, pinning the commit ceiling below what every other gauge
// assumes. Thresholds match the spec's own numbers: <15% free is its
// documented "crash risk" line; <8% is the free-space level the spec's
// source incident actually crashed at (20.1 GB / 446 GB ≈ 4.5%, but 8% is
// used here as a slightly less alarmist error line — the spec's own
// worked example put "safe again" at ≥60-80 GB free on a ~450 GB volume,
// i.e. ~13-18%, so 15%/8% brackets warning vs. already-in-the-danger-zone).
// A FIXED-size page file isn't gated by free disk this way — never colored.
function pagefileDiskColor(freePct: number | null, systemManaged: boolean): string {
    if (freePct == null || !systemManaged) return "var(--secondary-text-color)";
    if (freePct < 8) return "var(--error-color)";
    if (freePct < 15) return "var(--warning-color)";
    return "var(--secondary-text-color)";
}

const SystemStats = (): JSX.Element => {
    const [stats, setStats] = createSignal<SysStats | null>(null);
    // Per-volume list — feeds the Disk readout's tooltip (naming the drive
    // the % refers to) and is re-parsed live inside DiskVolumesPopover.
    const [diskVolumes, setDiskVolumes] = createSignal<DiskVolume[]>([]);

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

    // Per-drive Disk panel — same interaction as the CPU panel above.
    const [diskPanelOpen, setDiskPanelOpen] = createSignal(false);
    const [diskAnchorRect, setDiskAnchorRect] = createSignal<DOMRect | null>(null);
    let diskButtonRef: HTMLButtonElement | undefined;
    let diskPopoverRef: HTMLDivElement | undefined;

    const toggleDiskPanel = () => {
        if (diskPanelOpen()) {
            setDiskPanelOpen(false);
            return;
        }
        if (diskButtonRef) setDiskAnchorRect(diskButtonRef.getBoundingClientRect());
        setDiskPanelOpen(true);
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

    createEffect(() => {
        if (!diskPanelOpen()) return;
        const onDown = (e: MouseEvent) => {
            const t = e.target as Node;
            if (diskButtonRef?.contains(t) || diskPopoverRef?.contains(t)) return;
            setDiskPanelOpen(false);
        };
        const onKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                e.stopPropagation();
                setDiskPanelOpen(false);
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
                    pagefileVolumeFreeGb: vals["disk:pagefile_volume:free_gb"] ?? null,
                    pagefileVolumeFreePct: vals["disk:pagefile_volume:free_pct"] ?? null,
                    pagefileSystemManaged: (vals["disk:pagefile_system_managed"] ?? 0) > 0,
                });
                setDiskVolumes(parseDiskVolumes(vals));
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
                            data-tip="Commit charge used and total (RAM + page file budget)."
                            aria-label="Commit charge"
                        >
                            PF {formatMemBytes(s().commitUsed)}/{formatMemBytes(s().commitTotal)}
                        </span>
                    </Show>
                    {/* Free-space share of the system drive (the volume Windows backs
                        its page file with — SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29
                        §5.2 P0 is why THIS volume is the one watched, and the color
                        thresholds still encode that risk). The tooltip deliberately
                        explains only what the number is — free ÷ capacity, with live
                        figures — the page-file significance belongs to the PF gauge's
                        own tooltip. Click opens the per-drive breakdown (all volumes,
                        free space each), mirroring the CPU per-core panel. Only
                        rendered once the backend has a reading (Windows-only gauge;
                        absent elsewhere). */}
                    <Show when={s().pagefileVolumeFreePct != null}>
                        <span class="stat-separator">|</span>
                        <button
                            type="button"
                            ref={diskButtonRef}
                            class="stat-mono stat-pagefile-disk stat-disk-button"
                            style={{ color: pagefileDiskColor(s().pagefileVolumeFreePct, s().pagefileSystemManaged) }}
                            onClick={toggleDiskPanel}
                            data-tip={diskTooltip(diskVolumes())}
                            aria-label="Free disk space, click for per-drive breakdown"
                            aria-haspopup="dialog"
                            aria-expanded={diskPanelOpen()}
                        >
                            Disk {Math.round(s().pagefileVolumeFreePct!)}%
                        </button>
                        <Show when={diskPanelOpen()}>
                            <Portal>
                                <DiskVolumesPopover
                                    anchorRect={diskAnchorRect()}
                                    ref={(el) => { diskPopoverRef = el; }}
                                />
                            </Portal>
                        </Show>
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
