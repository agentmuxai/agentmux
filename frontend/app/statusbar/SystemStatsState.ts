// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The status bar's system readouts, the state: the latest system-info reading, the drives in it, and whether the
// per-core CPU and per-drive Disk panels are open.

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { createSignal, onCleanup, onMount } from "solid-js";
import { parseDiskVolumes, type DiskVolume } from "./disk-volumes";

export type SysStats = {
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

export type SystemStatsState = ReturnType<typeof createSystemStats>;

export function createSystemStats() {
    const [stats, setStats] = createSignal<SysStats | null>(null);
    // Per-volume list — feeds the Disk readout's tooltip (naming the drive
    // the % refers to) and is re-parsed live inside DiskVolumesPopover.
    const [diskVolumes, setDiskVolumes] = createSignal<DiskVolume[]>([]);

    // Per-core CPU and per-drive Disk panels, opened by clicking the
    // readout. Dismiss and positioning are AnchoredPopover's.
    const [cpuPanelOpen, setCpuPanelOpen] = createSignal(false);
    const toggleCpuPanel = () => setCpuPanelOpen(!cpuPanelOpen());

    const [diskPanelOpen, setDiskPanelOpen] = createSignal(false);
    const toggleDiskPanel = () => setDiskPanelOpen(!diskPanelOpen());

    onMount(() => {
        const unsub = onMuxEvent(
            WpsEvent.SysInfo,
            (data) => {
                const vals = data.values;
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
                        vals["disk:pagefile_system_managed"] != null ? vals["disk:pagefile_system_managed"] > 0 : null,
                });
                setDiskVolumes(parseDiskVolumes(vals));
            },
            "local"
        );
        onCleanup(() => unsub?.());
    });

    return {
        stats,
        diskVolumes,
        cpuPanelOpen,
        setCpuPanelOpen,
        toggleCpuPanel,
        diskPanelOpen,
        setDiskPanelOpen,
        toggleDiskPanel,
    };
}
