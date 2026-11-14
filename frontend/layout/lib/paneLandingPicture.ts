// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The picture a dragged tab's landing place shows (PaneLanding.tsx), one per drag. It is taken when the tab's
 * placing first starts (pillPlacement.ts), of the tab as it stands in its own pane then: the picture taken at
 * the press (tearoff-snapshot.ts) can be of a tab the press itself had only just brought to the front, a
 * Files tab still listing nothing. Where the pane cannot be pictured now, the press's picture stands in.
 *
 * Asked for once per drag: a placing ends and begins again many times in one drag, and from the second on the
 * layout no longer shows the tab as it stood.
 */

import { onSessionEnded } from "@/app/drag/drag-session";
import { peekTearOffSnapshot, picturePaneNow } from "@/app/drag/tearoff-snapshot";

/** A pane's picture (a base64 JPEG) and the pane's size when it was taken, in CSS px. */
export interface LandingPicture {
    picture: string;
    size: { width: number; height: number };
}

/** How long the press's picture is waited for when it is still being taken. */
const PRESS_PICTURE_WAIT_MS = 1500;

let kept: { blockId: string; shot: Promise<LandingPicture | undefined> } | undefined;
onSessionEnded(() => {
    kept = undefined;
});

async function take(blockId: string): Promise<LandingPicture | undefined> {
    try {
        const got = (await picturePaneNow(blockId)) ?? (await peekTearOffSnapshot(blockId, PRESS_PICTURE_WAIT_MS));
        // A picture of nothing is no picture.
        return got && got.size.width > 0 && got.size.height > 0 ? { picture: got.picture, size: got.size } : undefined;
    } catch {
        return undefined;
    }
}

/** The landing picture of tab `blockId` for the drag that is on: taken at the first asking, the same one after. */
export function landingPictureOf(blockId: string): Promise<LandingPicture | undefined> {
    if (kept?.blockId !== blockId) kept = { blockId, shot: take(blockId) };
    return kept.shot;
}

/** Test hook. */
export function resetLandingPictureForTests(): void {
    kept = undefined;
}
