// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import { cpuColor } from "./cpu-color";
import { CpuCoresPopover } from "./CpuCoresPopover";
import { diskTooltip, watchVolumeColor } from "./disk-volumes";
import { DiskVolumesPopover } from "./DiskVolumesPopover";
import { GpuReadout, MemReadouts, RateReadouts } from "./SystemStatsParts";
import { createSystemStats } from "./SystemStatsState";

const SystemStats = (): JSX.Element => {
    const state = createSystemStats();
    const {
        stats,
        diskVolumes,
        cpuPanelOpen,
        setCpuPanelOpen,
        toggleCpuPanel,
        diskPanelOpen,
        setDiskPanelOpen,
        toggleDiskPanel,
    } = state;
    let cpuButtonRef: HTMLButtonElement | undefined;
    let diskButtonRef: HTMLButtonElement | undefined;

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
                    <GpuReadout s={s} />
                    <MemReadouts s={s} />
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
                    <RateReadouts s={s} />
                </div>
            )}
        </Show>
    );
};

SystemStats.displayName = "SystemStats";

export { SystemStats };
