// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GPU / WebGL capability probe. Runs in the renderer with a throwaway <canvas>;
// safe to call anywhere and cached after the first call.
//
// Why this exists: when Chromium's GPU process can't initialize a hardware GL
// device (virtual display adapters such as Parsec, headless/RDP sessions, broken
// drivers) it disables GL entirely and WebGL becomes unavailable — xterm then
// falls back to its slower DOM renderer (no scrollbar). This probe surfaces that
// state for the status bar so the degraded mode is visible rather than silent.
// See docs/analysis/ANALYSIS_WEBGL_GPU_DISABLED_ROOTCAUSE_2026_06_10.md.

export type GpuClass = "hardware" | "software" | "unavailable";

export type GpuInfo = {
    webgl: boolean;
    webgl2: boolean;
    vendor: string | null; // unmasked GL vendor, when WEBGL_debug_renderer_info is available
    renderer: string | null; // unmasked GL renderer (e.g. "ANGLE (NVIDIA ...)" or "SwiftShader")
    classification: GpuClass;
};

// Substrings (lowercased) that mark a software/virtual GL backend rather than a
// real GPU. SwiftShader is Chromium's software rasterizer; llvmpipe is Mesa's;
// "Microsoft Basic Render" / "basic render driver" is the Windows fallback.
const SOFTWARE_MARKERS = [
    "swiftshader",
    "software",
    "llvmpipe",
    "microsoft basic render",
    "basic render driver",
    "disabled",
];

function classify(supported: boolean, renderer: string | null): GpuClass {
    if (!supported) return "unavailable";
    const r = (renderer ?? "").toLowerCase();
    if (SOFTWARE_MARKERS.some((m) => r.includes(m))) return "software";
    return "hardware";
}

let cached: GpuInfo | undefined;

function probe(): GpuInfo {
    let webgl = false;
    let webgl2 = false;
    let vendor: string | null = null;
    let renderer: string | null = null;
    try {
        const canvas = document.createElement("canvas");
        const gl2 = canvas.getContext("webgl2") as WebGL2RenderingContext | null;
        const gl = (gl2 ?? canvas.getContext("webgl")) as WebGLRenderingContext | null;
        webgl2 = !!gl2;
        webgl = !!gl;
        if (gl) {
            const dbg = gl.getExtension("WEBGL_debug_renderer_info");
            if (dbg) {
                vendor = gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL) as string;
                renderer = gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) as string;
            } else {
                vendor = gl.getParameter(gl.VENDOR) as string;
                renderer = gl.getParameter(gl.RENDERER) as string;
            }
        }
    } catch {
        // leave defaults → classified "unavailable"
    }
    return { webgl, webgl2, vendor, renderer, classification: classify(webgl, renderer) };
}

/** WebGL/GPU capability of the renderer process. Cached after first call. */
export function getGpuInfo(): GpuInfo {
    if (cached === undefined) cached = probe();
    return cached;
}

/** Short human label for a GPU classification. */
export function gpuClassLabel(c: GpuClass): string {
    switch (c) {
        case "hardware":
            return "Hardware";
        case "software":
            return "Software";
        default:
            return "Unavailable";
    }
}

/** Compact status-bar badge text, e.g. "HW" / "SW" / "off". */
export function gpuClassBadge(c: GpuClass): string {
    switch (c) {
        case "hardware":
            return "HW";
        case "software":
            return "SW";
        default:
            return "off";
    }
}
