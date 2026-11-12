// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The picture that follows the cursor during a window-tab or pane drag, on Windows
 * (crates/cef/src/commands/live_follow.rs; docs/reports/REPORT_LIVE_TEAROFF_CHROME_STYLE_2026_11_04.md §5).
 * The drag is unchanged and nothing is torn off until release; the host's mouse hook moves the picture (a
 * window tab's once the cursor is below the tab strip, a pane's from its first move), and the real window
 * opens where it was last seen. The host drops the picture by itself when the drag ends, after a grace for
 * the window to open; `endLiveFollow` drops it as soon as the real window is up, or when the drag ends
 * with no window to open.
 */

import { Logger } from "@/util/logger";
import { isWindows } from "@/util/platformutil";

type Api = Pick<AppApi, "liveFollowBegin" | "liveFollowEnd" | "liveFollowResize" | "liveFollowCapture">;

export interface LiveFollowSpec {
    sourceWindowLabel: string;
    /** Base64 JPEG of the window. */
    picture: string;
    /** The window's outer size, DIP. */
    windowWidth: number;
    windowHeight: number;
    /** Cursor to the window's top-left, DIP. */
    offsetX: number;
    offsetY: number;
    /** The viewport y below which a window tab's picture shows, DIP; none for a pane, shown everywhere from its first move. */
    tearLineY?: number;
}

/** Hand the host the picture and where it goes. A failure only means no picture: the tear-off is unchanged. */
export async function beginLiveFollow(api: Api, spec: LiveFollowSpec): Promise<void> {
    if (!isWindows()) return;
    try {
        await api.liveFollowBegin(spec);
    } catch (e) {
        Logger.warn("dnd", "live follow begin failed", { error: String(e) });
    }
}

/** The real window is up (or the tear-off failed): drop the picture now. */
export function endLiveFollow(api: Api): void {
    if (!isWindows()) return;
    api.liveFollowEnd().catch((e) => Logger.debug("dnd", "live follow end failed", { error: String(e) }));
}

/** The landing rect a size is for (`landingKey`), and whether the picture just taken of the pane there is kept (`adopt`). */
export interface LandingPicture {
    landingKey: string;
    adopt?: boolean;
}

/**
 * Give the picture another size, DIP. For a landing rect the host has the pane's picture of (`captureLiveFollow`),
 * it shows that picture; otherwise it resizes the picture it was first given. Calls go out in order, so a later
 * size can't land before an earlier one. A failure only leaves the old size.
 */
let resizes: Promise<unknown> = Promise.resolve();
export function resizeLiveFollow(
    api: Pick<Api, "liveFollowResize">,
    size: { width: number; height: number },
    landing?: LandingPicture
): Promise<void> {
    if (!isWindows()) return Promise.resolve();
    const next = resizes.then(() =>
        api.liveFollowResize({ windowWidth: Math.round(size.width), windowHeight: Math.round(size.height), ...landing })
    );
    resizes = next.catch((e) => Logger.debug("dnd", "live follow resize failed", { error: String(e) }));
    return resizes as Promise<void>;
}

export interface LandingCapture {
    landingKey: string;
    /** The size the picture is shown at for this landing rect, DIP. */
    size: { width: number; height: number };
    /** The pane on screen now, in the viewport's CSS px, and the viewport's width. */
    rect: { left: number; top: number; width: number; height: number };
    viewportWidth: number;
    /** A flat colour drawn over the pane, taken back out of its picture. */
    wash?: { r: number; g: number; b: number; a: number };
}

/**
 * Have the host take the pane's picture where it stands now, for the landing rect `landingKey`. The host cuts
 * it out of a picture of the window, so nothing is decoded or drawn here. True when a picture was taken and is
 * waiting to be kept (`resizeLiveFollow` with `adopt`); false when the host has one already, has no picture
 * following, or failed. One at a time: a second waits for the first.
 */
let captures: Promise<unknown> = Promise.resolve();
export function captureLiveFollow(api: Pick<Api, "liveFollowCapture">, capture: LandingCapture, wanted: () => boolean = () => true): Promise<boolean> {
    if (!isWindows()) return Promise.resolve(false);
    const next = captures.then(async () => {
        // Asked for while another was being taken, and no longer wanted by the time its turn came.
        if (!wanted()) return false;
        const got = await api.liveFollowCapture({
            landingKey: capture.landingKey,
            windowWidth: Math.round(capture.size.width),
            windowHeight: Math.round(capture.size.height),
            rect: capture.rect,
            viewportWidth: capture.viewportWidth,
            ...(capture.wash ? { wash: capture.wash } : {}),
        });
        return !!got?.staged;
    });
    const settled = next.catch((e) => {
        Logger.debug("dnd", "live follow capture failed", { error: String(e) });
        return false;
    });
    captures = settled;
    return settled;
}
