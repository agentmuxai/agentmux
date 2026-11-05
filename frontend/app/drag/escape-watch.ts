// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Escape during a pane or pane-tab drag, on Windows. A window-tab drag already has the host's hook
 * (droppable-tab.tsx); these two have none, and the page can't see the key during the OS drag. So the drag's
 * start arms a hook that reports only Escape (`tabdrag:escape-pressed`), which host-escape.ts keeps for
 * native-cancel.ts. The hook ends itself on mouse-up or Escape; the session's end stops it as a safety net.
 */

import { getApi } from "@/store/global";
import { Logger } from "@/util/logger";
import { isWindows } from "@/util/platformutil";
import { onSessionEnded } from "./drag-session";
import { noteHostEscape } from "./host-escape";
import { followPaneDrag } from "./pane-live-follow";

let listening = false;
let watching = false;
/** Start and stop calls go out in order: a stop for one drag must not land after the next drag's start. */
let calls: Promise<unknown> = Promise.resolve();

function inOrder(call: () => Promise<unknown>): void {
    calls = calls.then(call).catch((e) => Logger.warn("dnd", "escape watch call failed", { error: String(e) }));
}

/**
 * Arm the Escape hook for the drag that just began. A no-op off Windows. With `follow`, the same hook also
 * moves the picture of that pane while the cursor is outside every window (pane-live-follow.ts).
 */
export function watchEscapeDuringDrag(follow?: { blockId: string }): void {
    if (!isWindows()) return;
    if (!listening) {
        listening = true;
        void getApi()
            .listen("tabdrag:escape-pressed", () => noteHostEscape())
            .catch(() => {
                listening = false;
            });
    }
    watching = true;
    inOrder(async () => {
        const sourceWindowLabel = await getApi().getWindowLabel();
        await getApi().startTabDragTracking({ sourceWindowLabel, escapeOnly: true, ...(follow ? { paneFollow: true } : {}) });
        if (follow) void followPaneDrag(follow.blockId, sourceWindowLabel);
    });
}

onSessionEnded(() => {
    if (!watching) return;
    watching = false;
    inOrder(() => getApi().stopTabDragTracking());
});
