// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What stands in the landing place of a tab pill that leaves other tabs behind (pillPlacement.ts). A pane's
 * only tab moves the pane, so its landing place holds the pane itself. A tab of several has no pane of its own
 * until it is released: the layout opens a place for it and had nothing to put there, a bare gap now that the
 * landing zone draws nothing under the live reflow and the cursor carries no picture over the layout (the
 * operator, 2026-11-13: "I saw the other panes moving, but the dragged part is just an empty space").
 *
 * So the place holds the tab's own picture, the one taken as the pill was pressed (tearoff-snapshot.ts): the
 * pane as it stood, at its own size, from its top left corner, cut to the place, as a pane that moves is shown
 * until it is laid out again. Where there is no picture (none is taken off Windows, a background tab's, one
 * that failed) the place is an empty pane: the pane's own colour and the faint mark.
 *
 * Nothing a pointer or a screen reader meets: the drag announces itself.
 */

import { Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { onSessionEnded } from "@/app/drag/drag-session";
import { peekTearOffSnapshot, type PaneFollowPicture } from "@/app/drag/tearoff-snapshot";
import { EmptyPaneLogo } from "@/app/element/EmptyPaneLogo";

/** How long the place waits for a picture still being taken; the empty pane stands there meanwhile. */
const PICTURE_WAIT_MS = 1500;

/**
 * The picture for the drag that is on, asked for once: a placing ends and begins again many times in one drag,
 * and a picture is only handed out for its first seconds (tearoff-snapshot.ts `MAX_AGE_MS`).
 */
let kept: { blockId: string; shot: Promise<PaneFollowPicture | undefined> } | undefined;
onSessionEnded(() => {
    kept = undefined;
});

function pictureOf(blockId: string): Promise<PaneFollowPicture | undefined> {
    if (kept?.blockId !== blockId) {
        kept = { blockId, shot: peekTearOffSnapshot(blockId, PICTURE_WAIT_MS).catch(() => undefined) };
    }
    return kept.shot;
}

/** Test hook. */
export function resetPaneLandingForTests(): void {
    kept = undefined;
}

export const PaneLanding = (props: { blockId: string }): JSX.Element => {
    const [shot, setShot] = createSignal<PaneFollowPicture | undefined>(undefined);
    createEffect(() => {
        let mounted = true;
        void pictureOf(props.blockId).then((got) => {
            // A picture of nothing (a tab that was not the one shown) is no picture.
            if (mounted && got && got.size.width > 0 && got.size.height > 0) setShot(got);
        });
        onCleanup(() => {
            mounted = false;
        });
    });
    return (
        <div class="pane-landing" aria-hidden="true">
            <Show when={shot()} fallback={<EmptyPaneLogo />}>
                {(got) => (
                    <img
                        src={`data:image/jpeg;base64,${got().picture}`}
                        style={{ width: `${got().size.width}px`, height: `${got().size.height}px` }}
                        alt=""
                        draggable={false}
                    />
                )}
            </Show>
        </div>
    );
};
