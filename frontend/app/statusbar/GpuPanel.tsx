// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * GpuPanel: the click-opened panel under the status bar's GPU readouts (the "GPU n%" meter and "GFX HW"): the
 * adapters' use and memory, and which processes use the GPU, AgentMux's own named as Tower names them
 * (SPEC_STATUS_BAR_GPU_PANEL_2026_11_15.md). Inline through AnchoredPopover, like the CPU and Disk panels: no modal.
 * The figures are srv's (`gpu.sample`), asked for only while this is mounted.
 */

import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { t } from "@/app/i18n/i18n";
import { openPane } from "@/app/store/palette-open-pane";
import { getGpuInfo } from "@/util/gpuutil";
import { type JSX } from "solid-js";
import { createGpuSample } from "./gpu-panel-state";
import { AdapterList, GfxLine, GpuFooter, GpuNotice, ProcessList } from "./GpuPanelParts";

interface GpuPanelProps {
    /** The readout it opens from. */
    anchor: PopoverAnchor;
    /** This window's page's busy share, for the footer (`ui-busy.ts`). */
    uiBusy: number | null;
    onClose: () => void;
}

/** Tower's Processes view, showing AgentMux's own processes (the task Tower groups them in). */
function openTower(): void {
    // Tower's own keys are not in the typed meta: it reads them as it does a restored pane's.
    const meta: Record<string, unknown> = { view: "tower", "tower:view": "processes", "tower:only": "agentmux" };
    void openPane({ meta: meta as BlockDef["meta"] });
}

export const GpuPanel = (props: GpuPanelProps): JSX.Element => {
    const { sample, failed } = createGpuSample();
    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-start"
            onDismiss={props.onClose}
            class="gpu-panel"
            role="dialog"
            aria-label={t("gpuPanel.aria")}
        >
            <div class="gpu-panel-header">
                <span class="gpu-panel-title">{t("gpuPanel.title")}</span>
            </div>
            <GfxLine info={getGpuInfo()} />
            <GpuNotice sample={sample()} failed={failed()} />
            <AdapterList adapters={sample()?.adapters ?? []} />
            {sample() && (
                <ProcessList
                    sample={sample()!}
                    onOpenTower={() => {
                        props.onClose();
                        openTower();
                    }}
                />
            )}
            <GpuFooter sample={sample()} uiBusy={props.uiBusy} />
        </AnchoredPopover>
    );
};

GpuPanel.displayName = "GpuPanel";
