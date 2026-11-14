// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What stands in the landing place of a tab pill that leaves other tabs behind (pillPlacement.ts). A pane's
 * only tab moves the pane, so its landing place holds the pane itself. A tab of several has no pane of its own
 * until it is released: the layout opens a place for it and had nothing to put there, a bare gap now that the
 * landing zone draws nothing under the live reflow and the cursor carries no picture over the layout (the
 * operator, 2026-11-13: "I saw the other panes moving, but the dragged part is just an empty space").
 *
 * So the place holds the tab's own picture (paneLandingPicture.ts), resized to the whole place. It is a
 * picture, not the pane laid out for the place: a place of another shape shows it stretched. Where there is no
 * picture (the pane could not be pictured, a host that takes none) the place is an empty pane: the pane's own
 * colour and the faint mark.
 *
 * Nothing a pointer or a screen reader meets: the drag announces itself.
 */

import { Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { EmptyPaneLogo } from "@/app/element/EmptyPaneLogo";
import { landingPictureOf, type LandingPicture } from "./paneLandingPicture";

export const PaneLanding = (props: { blockId: string }): JSX.Element => {
    const [shot, setShot] = createSignal<LandingPicture | undefined>(undefined);
    createEffect(() => {
        let mounted = true;
        void landingPictureOf(props.blockId).then((got) => {
            if (mounted && got) setShot(got);
        });
        onCleanup(() => {
            mounted = false;
        });
    });
    return (
        <div class="pane-landing" aria-hidden="true">
            <Show when={shot()} fallback={<EmptyPaneLogo />}>
                {(got) => <img src={`data:image/jpeg;base64,${got().picture}`} alt="" draggable={false} />}
            </Show>
        </div>
    );
};
