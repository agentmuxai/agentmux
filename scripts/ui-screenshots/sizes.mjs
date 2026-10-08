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
// PNG (2 = sharp on high-DPI screens, at four times the file size).

export const SIZES = {
    small: { width: 800, height: 600, scale: 1 },
    medium: { width: 1280, height: 800, scale: 1 },
    large: { width: 1920, height: 1080, scale: 1 },
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
