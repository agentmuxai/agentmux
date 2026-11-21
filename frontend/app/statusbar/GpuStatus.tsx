// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { termRendererAtom } from "@/store/global";
import { getGpuInfo, gpuClassBadge } from "@/util/gpuutil";
import { Show, type JSX } from "solid-js";
import { closeGpuPanel, gpuPanelAnchor, toggleGpuPanel } from "./gpu-panel-state";
import { GpuPanel } from "./GpuPanel";
import { sharedUiBusy } from "./ui-busy";

// Compact bottom-left status-bar line for GPU/WebGL state (enabled/disabled). A click opens the GPU panel (use and
// memory, per adapter and per process; GpuPanel.tsx), the same one the "GPU n%" meter beside CPU opens. Driver detail
// lives in the BackendStatus popover (click the uptime). The probe is in gpuutil.ts.
const GpuStatus = (): JSX.Element => {
    const info = getGpuInfo();
    let ref: HTMLButtonElement | undefined;

    const color = () => {
        switch (info.classification) {
            case "hardware":
                return "var(--secondary-text-color)";
            case "software":
                return "var(--warning-text-color)";
            default:
                return "var(--error-text-color)";
        }
    };

    const tip = () => {
        const term = termRendererAtom();
        const rend = info.renderer ? ` — ${info.renderer}` : "";
        const termPart = term ? ` · terminal: ${term.toUpperCase()}` : "";
        switch (info.classification) {
            case "hardware":
                return `Graphics: hardware accelerated${rend}${termPart}`;
            case "software":
                return `Graphics: software rendering${rend}${termPart}`;
            default:
                return `Graphics: GPU disabled — WebGL unavailable, terminals use the DOM renderer${termPart}`;
        }
    };

    // Labeled "GFX" (graphics-acceleration state) to distinguish from SystemStats'
    // "GPU {n}%" (utilization) sitting in the same bar.
    return (
        <>
            <Button
                variant="chrome"
                ref={ref}
                class="stat-mono stat-gpu-mode stat-gfx-button"
                style={{ color: color() }}
                data-tip={tip()}
                aria-label={t("gpuMeter.gfxAria")}
                aria-haspopup="dialog"
                aria-expanded={ref != null && gpuPanelAnchor() === ref}
                onClick={() => ref && toggleGpuPanel(ref)}
            >
                GFX {gpuClassBadge(info.classification)}
            </Button>
            <Show when={ref != null && gpuPanelAnchor() === ref}>
                <GpuPanel anchor={ref} uiBusy={sharedUiBusy()()} onClose={closeGpuPanel} />
            </Show>
        </>
    );
};

GpuStatus.displayName = "GpuStatus";

export { GpuStatus };
