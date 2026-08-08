// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DiskVolumesPopover — click-opened panel anchored under the status-bar Disk
 * readout. Lists every mounted volume with its free space. The backend
 * publishes per-volume capacity in the same `sysinfo`/`local` event the rest
 * of the status bar consumes (`disk:vol:<mount>:free_gb` / `:total_gb`, see
 * sysinfo.rs::get_disk_data), so this is a pure-frontend view — the sibling
 * of CpuCoresPopover's per-core panel, sharing its positioning + airspace
 * pattern (usePaneOverlay, computeMenuPosition top-end, autoUpdate).
 */

import { createSignal, Index, onCleanup, onMount, Show, type JSX } from "solid-js";
import { autoUpdate } from "@floating-ui/dom";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { computeMenuPosition } from "@/app/util/menu-position";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { diskFreeColor, formatDiskGb, parseDiskVolumes, type DiskVolume } from "./disk-volumes";

interface DiskVolumesPopoverProps {
    anchorRect: DOMRect | null;
    ref?: (el: HTMLDivElement) => void;
}

export const DiskVolumesPopover = (props: DiskVolumesPopoverProps): JSX.Element => {
    let rootRef: HTMLDivElement | undefined;

    // Airspace cut so the popover paints over any browser-pane HWND the status
    // bar overlaps — same primitive as CpuCoresPopover / TokenBreakdownPopover.
    usePaneOverlay(() => rootRef);

    const [volumes, setVolumes] = createSignal<DiskVolume[]>([]);

    onMount(() => {
        const unsub = waveEventSubscribe({
            eventType: WpsEvent.SysInfo,
            scope: "local",
            handler: (event) => {
                const vals = (event as WaveEvent)?.data?.values;
                if (vals == null) return;
                setVolumes(parseDiskVolumes(vals));
            },
        });
        onCleanup(() => unsub?.());
    });

    // ── Positioning (mirrors CpuCoresPopover / TokenBreakdownPopover) ────────
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });
    let cleanupAutoUpdate: (() => void) | null = null;

    const registerFloating = (el: HTMLDivElement) => {
        rootRef = el;
        props.ref?.(el);
        requestAnimationFrame(() => {
            const r = props.anchorRect;
            if (!r || !(el instanceof Element)) return;
            const update = async () => {
                const cur = props.anchorRect;
                if (!cur) return;
                const pos = await computeMenuPosition({ anchor: cur, placement: "top-end", avoidNativePanes: false }, el);
                setFloatingStyle(pos.style);
            };
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(
                { getBoundingClientRect: () => props.anchorRect ?? r },
                el,
                update,
            );
        });
    };

    onCleanup(() => cleanupAutoUpdate?.());

    const usedPct = (v: DiskVolume): number => {
        if (v.totalGb <= 0) return 0;
        return Math.min(100, Math.max(0, ((v.totalGb - v.freeGb) / v.totalGb) * 100));
    };

    return (
        <div
            ref={registerFloating}
            class="disk-volumes-popover"
            role="dialog"
            aria-label="Free space per disk drive"
            data-pane-overlay
            style={{ ...floatingStyle(), width: "300px" }}
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
        </div>
    );
};

DiskVolumesPopover.displayName = "DiskVolumesPopover";
