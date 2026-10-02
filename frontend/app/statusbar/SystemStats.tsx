// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { CpuCoresPopover } from "./CpuCoresPopover";
import { DiskVolumesPopover } from "./DiskVolumesPopover";
import { cpuColor } from "./cpu-color";
import { diskTooltip, parseDiskVolumes, watchVolumeColor, type DiskVolume } from "./disk-volumes";

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
    watchVolumeFreeGb: number | null;
    watchVolumeFreePct: number | null;
    /** null off Windows — see watchVolumeColor's tri-state. */
    pagefileSystemManaged: boolean | null;
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
    // Per-volume list — feeds the Disk readout's tooltip (naming the drive
    // the % refers to) and is re-parsed live inside DiskVolumesPopover.
    const [diskVolumes, setDiskVolumes] = createSignal<DiskVolume[]>([]);

    // Per-core CPU and per-drive Disk panels, opened by clicking the
    // readout. Dismiss and positioning are AnchoredPopover's.
    const [cpuPanelOpen, setCpuPanelOpen] = createSignal(false);
    let cpuButtonRef: HTMLButtonElement | undefined;
    const toggleCpuPanel = () => setCpuPanelOpen(!cpuPanelOpen());

    const [diskPanelOpen, setDiskPanelOpen] = createSignal(false);
    let diskButtonRef: HTMLButtonElement | undefined;
    const toggleDiskPanel = () => setDiskPanelOpen(!diskPanelOpen());

    onMount(() => {
        const unsub = muxEventSubscribe({
            eventType: WpsEvent.SysInfo,
            scope: "local",
            handler: (event) => {
                const vals = (event as MuxEvent)?.data?.values;
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
                    watchVolumeFreeGb: vals["disk:watch:free_gb"] ?? null,
                    watchVolumeFreePct: vals["disk:watch:free_pct"] ?? null,
                    // Presence, not truthiness: an ABSENT key means "not
                    // Windows", which is a different state from an explicit
                    // false (fixed-size page file). Collapsing them mutes the
                    // pill everywhere off Windows.
                    pagefileSystemManaged:
                        vals["disk:pagefile_system_managed"] != null
                            ? vals["disk:pagefile_system_managed"] > 0
                            : null,
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
                        <CpuCoresPopover anchor={cpuButtonRef} onClose={() => setCpuPanelOpen(false)} />
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
                    {/* Free-space share of the watched volume: the page-file drive on
                        Windows (SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 §5.2 P0 is why
                        THAT volume, and the color thresholds still encode that risk),
                        the mount backing the data dir elsewhere
                        (SPEC_STATUSBAR_DISK_PILL_CROSS_PLATFORM_2026_09_18). The
                        tooltip is a short label only
                        ("Free share of system drive (C:)"), matching the terse form
                        every other stat tooltip uses — no math, no live figures, and
                        no page-file mention (that significance belongs to the PF
                        gauge's own tooltip); see diskTooltip() in disk-volumes.ts for
                        the system-drive-vs-page-file-volume wording rule. Live figures
                        and the drive list live in the click-open panel, mirroring the
                        CPU per-core panel. Only rendered once the backend has a
                        reading — every platform emits one, but a torn first tick
                        may not. */}
                    <Show when={s().watchVolumeFreePct != null}>
                        <span class="stat-separator">|</span>
                        <button
                            type="button"
                            ref={diskButtonRef}
                            class="stat-mono stat-watch-disk stat-disk-button"
                            style={{ color: watchVolumeColor(s().watchVolumeFreePct, s().pagefileSystemManaged) }}
                            onClick={toggleDiskPanel}
                            data-tip={diskTooltip(diskVolumes())}
                            aria-label="Free disk space, click for per-drive breakdown"
                            aria-haspopup="dialog"
                            aria-expanded={diskPanelOpen()}
                        >
                            Disk {Math.round(s().watchVolumeFreePct!)}%
                        </button>
                        <Show when={diskPanelOpen()}>
                            <DiskVolumesPopover
                                anchor={diskButtonRef}
                                initialVolumes={diskVolumes()}
                                onClose={() => setDiskPanelOpen(false)}
                            />
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
