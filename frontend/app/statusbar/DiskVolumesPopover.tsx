// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DiskVolumesPopover — click-opened panel anchored under the status-bar Disk
 * readout. Lists every mounted volume with its free space. The backend
 * publishes per-volume capacity in the same `sysinfo`/`local` event the rest
 * of the status bar consumes (`disk:vol:<mount>:free_gb` / `:total_gb`, see
 * sysinfo.rs::get_disk_data), so this is a pure-frontend view — the sibling
 * of CpuCoresPopover's per-core panel, rendered through the same
 * AnchoredPopover but left-aligned
 * to the pill (top-start) rather than CPU's right-aligned (top-end) — per
 * request, the panel's left edge lines up with the pill's left edge.
 */

import { createSignal, Index, onCleanup, onMount, Show, type JSX } from "solid-js";
import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { diskFreeColor, formatDiskGb, parseDiskVolumes, type DiskVolume } from "./disk-volumes";

interface DiskVolumesPopoverProps {
    /** The status bar's Disk readout. */
    anchor: PopoverAnchor;
    onClose: () => void;
    /** Parent's latest parsed snapshot — seeds the list so opening the panel
     *  shows data immediately instead of "Reading drives…" until the next
     *  sysinfo tick (the MPS route suppresses event replay for this second
     *  subscription, so a fresh mount would otherwise start empty). */
    initialVolumes?: DiskVolume[];
}

export const DiskVolumesPopover = (props: DiskVolumesPopoverProps): JSX.Element => {
    const [volumes, setVolumes] = createSignal<DiskVolume[]>(props.initialVolumes ?? []);

    onMount(() => {
        const unsub = muxEventSubscribe({
            eventType: WpsEvent.SysInfo,
            scope: "local",
            handler: (event) => {
                const vals = (event as MuxEvent)?.data?.values;
                if (vals == null) return;
                setVolumes(parseDiskVolumes(vals));
            },
        });
        onCleanup(() => unsub?.());
    });


    const usedPct = (v: DiskVolume): number => {
        if (v.totalGb <= 0) return 0;
        return Math.min(100, Math.max(0, ((v.totalGb - v.freeGb) / v.totalGb) * 100));
    };

    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-start"
            onDismiss={props.onClose}
            class="disk-volumes-popover"
            role="dialog"
            aria-label="Free space per disk drive"
            style={{ width: "300px" }}
        >
            <div class="disk-volumes-header">
                <span class="disk-volumes-title">Disk Space</span>
                <span class="disk-volumes-count">
                    {volumes().length} {volumes().length === 1 ? "drive" : "drives"}
                </span>
            </div>

            <Show
                when={volumes().length > 0}
                fallback={<div class="disk-volumes-empty">Reading drives…</div>}
            >
                <div class="disk-volumes-list">
                    <Index each={volumes()}>
                        {(v) => (
                            <div class="disk-volume">
                                <span class="disk-volume-label">{v().label}</span>
                                <span class="disk-volume-bar" aria-hidden="true">
                                    <span
                                        class="disk-volume-bar-fill"
                                        style={{ width: `${usedPct(v())}%` }}
                                    />
                                </span>
                                <span
                                    class="disk-volume-free"
                                    style={{ color: diskFreeColor(v().freeGb, v().totalGb) }}
                                >
                                    {formatDiskGb(v().freeGb)} free of {formatDiskGb(v().totalGb)}
                                </span>
                            </div>
                        )}
                    </Index>
                </div>
            </Show>
        </AnchoredPopover>
    );
};

DiskVolumesPopover.displayName = "DiskVolumesPopover";
