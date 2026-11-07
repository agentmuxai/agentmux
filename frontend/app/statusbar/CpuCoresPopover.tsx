// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CpuCoresPopover — click-opened panel anchored under the status-bar CPU
 * readout. Shows live per-core CPU usage. The backend already publishes both
 * the aggregate (`cpu`) and every per-core value (`cpu:0`..`cpu:N`) in the
 * `sysinfo`/`local` event, so this is a pure-frontend view.
 *
 * The layout is adaptive (SPEC_STATUSBAR_CPU_CORES_PANEL_2026_06_15.md §4.3):
 *   ≤16 cores → labeled rows (label + bar + %)
 *   17–64     → compact cells (index + % + mini bar)
 *   65+       → heatmap of computed-size squares, detail on hover/focus
 *
 * Renders through AnchoredPopover (positioning top-end, chrome zoom, dismiss,
 * the airspace cut).
 */

import { Show, type JSX } from "solid-js";
import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { CoreHeatmap, CoreList, CpuHeader, CpuSubtitle } from "./CpuCoresPopoverParts";
import { createCpuCores } from "./CpuCoresPopoverState";

interface CpuCoresPopoverProps {
    /** The status bar's CPU readout. */
    anchor: PopoverAnchor;
    onClose: () => void;
}

export const CpuCoresPopover = (props: CpuCoresPopoverProps): JSX.Element => {
    const s = createCpuCores();

    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-end"
            onDismiss={props.onClose}
            class={`cpu-cores-popover cpu-cores-popover--${s.tier()}`}
            role="dialog"
            aria-label="Per-core CPU usage"
            style={{ width: `${s.panelWidth()}px` }}
        >
            <CpuHeader s={s} />
            <CpuSubtitle s={s} />

            <Show
                when={s.cores().length > 0}
                fallback={<div class="cpu-cores-empty">Reading CPU…</div>}
            >
                <CoreList s={s} />
                <CoreHeatmap s={s} />
            </Show>
        </AnchoredPopover>
    );
};

CpuCoresPopover.displayName = "CpuCoresPopover";
