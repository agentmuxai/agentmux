// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { termRendererAtom } from "@/store/global";
import { getGpuInfo, gpuClassBadge } from "@/util/gpuutil";
import { type JSX } from "solid-js";

// Compact bottom-left status-bar line for GPU/WebGL state. Detail lives in the
// BackendStatus popover (click the uptime). See gpuutil.ts for the probe and
// docs/analysis/ANALYSIS_WEBGL_GPU_DISABLED_ROOTCAUSE_2026_06_10.md for why this
// matters (silent DOM-renderer fallback when the GPU process can't start).
const GpuStatus = (): JSX.Element => {
    const info = getGpuInfo();

    const color = () => {
        switch (info.classification) {
            case "hardware":
                return "var(--secondary-text-color)";
            case "software":
                return "var(--warning-color)";
            default:
                return "var(--error-color)";
        }
    };

    const tip = () => {
        const term = termRendererAtom();
        const rend = info.renderer ? ` — ${info.renderer}` : "";
        const termPart = term ? ` · terminal: ${term.toUpperCase()}` : "";
        switch (info.classification) {
            case "hardware":
                return `GPU: hardware accelerated${rend}${termPart}`;
            case "software":
                return `GPU: software rendering (SwiftShader)${rend}${termPart}`;
            default:
                return `GPU: unavailable — WebGL disabled, terminals use the DOM renderer${termPart}`;
        }
    };

    return (
        <span
            class="stat-mono stat-gpu-mode"
            style={{ color: color() }}
            data-tip={tip()}
            aria-label="GPU rendering status"
        >
            GPU {gpuClassBadge(info.classification)}
        </span>
    );
};

GpuStatus.displayName = "GpuStatus";

export { GpuStatus };
