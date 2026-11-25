// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Spike of the host-armed tear-off picture (#48707, docs/specs/SPEC_TEAROFF_HOST_ARMED_PICTURE_2026_11_25.md): on
 * a press on a pane's header, ask the host to copy the window natively and log how long it took. Off unless the
 * window's `localStorage["debug:nativepicture"]` is "native" (the copy, beside the screenshot the press takes),
 * "compare" (also the CDP screenshot of the same moment, to check the pixels) or "only" (the copy instead of the
 * press's screenshot: what a click costs when the copy is the picture). The host keeps nothing of it: times and
 * sizes only, in its log.
 */

import { getApi } from "@/app/store/global";

const FLAG = "debug:nativepicture";

/** The mode the spike is switched to on this window, or none. */
function mode(): "native" | "compare" | "only" | "off" | undefined {
    try {
        const v = window.localStorage.getItem(FLAG);
        return v === "native" || v === "compare" || v === "only" || v === "off" ? v : undefined;
    } catch {
        return undefined;
    }
}

/** The window's flag is "off": a tear-off picture is the page's screenshot, not the host's native copy (to compare the two in one run). */
export function nativeCopyOff(): boolean {
    return mode() === "off";
}

/** A pane's header was pressed: ask for the native copy, without waiting for it. True when it stands in for the press's screenshot ("only"). */
export function probeNativePicture(): boolean {
    const m = mode();
    if (!m || m === "off") return false;
    const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
    getApi()
        .windows.nativePictureProbe(label, m === "compare")
        .catch(() => {});
    return m === "only";
}
