// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The picture of the dragged pane that the cursor carries during a pane drag, on Windows (drag/live-follow.ts,
 * and crates/cef/src/commands/live_follow.rs for the host half). It is the pane's own picture
 * (tearoff-snapshot.ts), at the pane's own size and shape, held at the grab the pointer pressed it with.
 *
 * It is not shown while the cursor is over the body of the layout the drag began in. There the layout itself
 * shows the pane: where it would land, as the other panes make room (liveReflow.ts), or its own place while a
 * release would tear it off. The hand holds nothing and the cursor alone says a drag is on (the operator,
 * 2026-11-13: "it does not need to show at all while a user is hovering over the main window tab body, just
 * show the moving cursor icon, the user already sees the placed pane. when going off window or into another
 * tab, thats when we want to see its original shape"). It shows everywhere else: over the window's tab bar,
 * outside every AgentMux window, where the drop tears the pane off and the floating window opens where the
 * picture was last seen, over the body of another window tab the drag has gone on to, whose layout shows
 * nothing of the pane, and over its own layout while that shows the tear-off (the pointer rested over the
 * pane's own place and the others closed over it), when the pane is in no place on screen. No translucent
 * ghost ever stands in for it. The picture stays on the monitor its window is on: it stops whole at that
 * monitor's edge, as a window does, while the cursor goes on (the host's `home`).
 *
 * It is never resized. (It took the landing rect's shape for a while, smaller than it, and then a second
 * picture of the pane laid out for that rect; the operator, 2026-11-13: "lets get rid of the minimized teared
 * off pane, keep it full size".)
 */

import { createEffect, createRoot } from "solid-js";
import { getApi } from "@/store/global";
import { isWindows } from "@/util/platformutil";
import { beginLiveFollow, fadeLiveFollow, quietLiveFollow } from "./live-follow";
import { peekTearOffSnapshot } from "./tearoff-snapshot";

/** How long the drag waits for the pane's picture; without one the pane tears off as before. */
const PICTURE_WAIT_MS = 1500;

/** The layout the drag began in: where its picture stays out of sight. */
export interface HomeLayout {
    /** The layout's body on screen now, in the viewport's CSS px; none when it cannot be read. */
    body: () => { left: number; top: number; width: number; height: number } | undefined;
    /**
     * Whether that layout shows the pane now: not while the window shows another window tab the drag has gone
     * on to, and not while the layout shows the tear-off, the others closed over the pane's place. Reactive.
     */
    holdsPane: () => boolean;
    /** Whether the drag is still on. Reactive. */
    active: () => boolean;
}

/**
 * Hand the host the pane's picture, once its drag hook is armed, with the body of its `home` layout as the area
 * it stays out of. Without `home` it shows everywhere. Never throws.
 */
export async function followPaneDrag(blockId: string, windowLabel: string, home?: HomeLayout): Promise<void> {
    const shot = await peekTearOffSnapshot(blockId, PICTURE_WAIT_MS);
    if (!shot) return;
    const body = home?.body();
    await beginLiveFollow(getApi(), {
        sourceWindowLabel: windowLabel,
        picture: shot.picture,
        windowWidth: shot.size.width,
        windowHeight: shot.size.height,
        offsetX: shot.grab.x,
        offsetY: shot.grab.y,
        ...(body ? { quiet: { ...body, viewportWidth: window.innerWidth } } : {}),
    });
    if (home && body) followHome(home);
}

/**
 * Fade the picture while a layout shows drop guides (`on`), and show it solid again when none does
 * (dropGuides.ts). The guides are in the page and the picture is over the page, at the pane's own size, so over
 * another window tab's layout it covered the spots a drop is chosen by. Faded, both are seen (the operator,
 * 2026-11-15: "when the follow pane is over the guide, can you make the follow pane 70% transparent?").
 */
export function fadeFollowedPane(on: boolean): void {
    if (!isWindows()) return;
    void fadeLiveFollow(getApi(), on);
}

/** Keep the quiet area on only while the home layout shows the pane, until the drag ends. */
function followHome(home: HomeLayout): void {
    createRoot((dispose) => {
        // The host starts with the area on: the drag began in the layout, with the pane in its place.
        let on = true;
        createEffect(() => {
            if (!home.active()) {
                dispose();
                return;
            }
            const next = home.holdsPane();
            if (next === on) return;
            on = next;
            void quietLiveFollow(getApi(), next);
        });
    });
}
