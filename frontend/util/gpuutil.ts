// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GPU / WebGL capability probe. Runs in the renderer with a throwaway <canvas>;
// safe to call anywhere and cached after the first call.
//
// Why this exists: when Chromium's GPU process can't initialize a hardware GL
// device, it disables GL and WebGL becomes unavailable — xterm then silently
// falls back to its slower DOM renderer. This probe drives the status-bar GPU
// indicator so that degraded state is visible (enabled/disabled + which GL
// renderer/driver is actually in use) rather than silent.

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
            // Release the probe context immediately. Chromium caps live WebGL
            // contexts per page (~16) and evicts the oldest; holding this one for
            // the renderer's lifetime would steal a slot from xterm's WebGL
            // terminals. We only needed it to read the capability params.
            gl.getExtension("WEBGL_lose_context")?.loseContext();
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
