// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a window-tab drag arms at its start: the host's mouse hook, and on Windows the picture of the window
 * that follows the cursor once the tab is below the strip (drag/live-follow.ts).
 */

import { beginLiveFollow } from "@/app/drag/live-follow";
import { peekWindowTabSnapshot } from "@/app/drag/tearoff-snapshot";
import { getApi } from "@/store/global";
import { hostHas } from "@/util/host-caps";
import { Logger } from "@/util/logger";
import { isWindows } from "@/util/platformutil";
import { tabAnchorOffset } from "./tab-grab-offset";
import { TEAR_PAST_PX } from "./tabbar-dnd";

/** How long the drag waits for the window's picture; without one the tab tears off as before. */
const PICTURE_WAIT_MS = 1500;

export interface WindowTabDrag {
    tabId: string;
    workspaceId: string;
    windowLabel: string;
    /** The only tab in its window: it never tears off, so there is nothing to show. */
    isLastTab: boolean;
    /** The dragged tab's wrapper, to find the strip. */
    tabEl: Element;
}

export async function armWindowTabDrag(drag: WindowTabDrag): Promise<void> {
    try {
        await getApi().startTabDragTracking({
            sourceWindowLabel: drag.windowLabel,
            tabId: drag.tabId,
            sourceWsId: drag.workspaceId,
            isLastTab: drag.isLastTab,
        });
    } catch (e) {
        Logger.warn("dnd", "startTabDragTracking failed", { error: String(e) });
        return;
    }
    if (isWindows() && hostHas("tearOff") && !drag.isLastTab) await followBelowStrip(drag);
}

async function followBelowStrip(drag: WindowTabDrag): Promise<void> {
    const strip = drag.tabEl.closest(".tab-bar-scroll");
    const offset = tabAnchorOffset(strip);
    const picture = await peekWindowTabSnapshot(drag.tabId, PICTURE_WAIT_MS);
    if (!strip || !offset || !picture) return;
    await beginLiveFollow(getApi(), {
        sourceWindowLabel: drag.windowLabel,
        picture,
        windowWidth: window.outerWidth,
        windowHeight: window.outerHeight,
        offsetX: offset.x,
        offsetY: offset.y,
        tearLineY: strip.getBoundingClientRect().bottom + TEAR_PAST_PX,
    });
}
