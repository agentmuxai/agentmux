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

type Api = Pick<AppApi, "liveFollowBegin" | "liveFollowEnd" | "liveFollowResize">;

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

/**
 * Give the picture another size, DIP: the host cuts or pads the picture it was given, never scales it. Calls
 * go out in order, so a later size can't land before an earlier one. A failure only leaves the old size.
 */
let resizes: Promise<unknown> = Promise.resolve();
export function resizeLiveFollow(api: Pick<Api, "liveFollowResize">, size: { width: number; height: number }): Promise<void> {
    if (!isWindows()) return Promise.resolve();
    const next = resizes.then(() =>
        api.liveFollowResize({ windowWidth: Math.round(size.width), windowHeight: Math.round(size.height) })
    );
    resizes = next.catch((e) => Logger.debug("dnd", "live follow resize failed", { error: String(e) }));
    return resizes as Promise<void>;
}
