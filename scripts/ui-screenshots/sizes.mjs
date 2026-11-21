// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Named window sizes for screenshot variants — see
// docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §8. A shot with
// `sizes` set is captured once per size: the page's viewport is set to that
// size with CDP's Emulation.setDeviceMetricsOverride (the real window is never
// resized, so the result doesn't depend on the screen it's on), the layout
// reflows, and the shot's selector is measured and cropped afresh.
//
// width/height are the whole window's CSS pixels; a maximized pane fills it
// less the top bar and status bar. `scale` is the device pixel ratio of the
// PNG: every size is captured at CAPTURE_SCALE (2), so a picture is crisp on a high-DPI screen at the width the
// page shows it; the file is four times the pixels (docs/specs/SPEC_WEBSITE_DOCS_MEDIA_REFRESH_2026_11_20.md §5).
import { CAPTURE_SCALE } from "./lib/crisp.mjs";


export const SIZES = {
    small: { width: 800, height: 600, scale: CAPTURE_SCALE },
    medium: { width: 1280, height: 800, scale: CAPTURE_SCALE },
    large: { width: 1920, height: 1080, scale: CAPTURE_SCALE },
};

/** Parses `--sizes small,large` (or `all`) against SIZES; throws on an unknown name. */
export function parseSizes(arg, sizes = SIZES) {
    if (!arg || arg === "all") return Object.keys(sizes);
    const names = arg
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
    // An empty selection ("--sizes ,", an empty variable) would capture nothing
    // and still report success.
    if (names.length === 0) throw new Error(`--sizes "${arg}" names no size (known: ${Object.keys(sizes).join(", ")})`);
    for (const n of names) {
        if (!sizes[n]) throw new Error(`unknown size "${n}" (known: ${Object.keys(sizes).join(", ")})`);
    }
    return names;
}

/** The output file name for a shot, and for one of its sizes. */
export function shotFilename(n, id, size) {
    return size ? `${id}-${size}.png` : `${n}-${id}.png`;
}
