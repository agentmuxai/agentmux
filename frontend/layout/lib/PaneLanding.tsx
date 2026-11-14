// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What stands in the landing place of a tab pill that leaves other tabs behind (pillPlacement.ts). A pane's
 * only tab moves the pane, so its landing place holds the pane itself. A tab of several has no pane of its own
 * until it is released: the layout opens a place for it and had nothing to put there, a bare gap now that the
 * landing zone draws nothing under the live reflow and the cursor carries no picture over the layout (the
 * operator, 2026-11-13: "I saw the other panes moving, but the dragged part is just an empty space").
 *
 * So the place holds the pane the release makes, inside the gap a pane keeps to its neighbours:
 * - the tab itself, laid out for the place as a dragged pane is: its own content under a header with only
 *   this tab (tabLanding.ts). It was a picture of the tab resized to the place, which a place of another
 *   shape showed stretched (the operator, 2026-11-13: "dragging a pane tab will get a distored look");
 * - where the tab's content cannot be moved there (a browser's page, a tab that is not mounted), its picture
 *   (paneLandingPicture.ts) at the picture's own size, from the place's top left corner: cut off where the
 *   place is smaller, the pane's colour where it is larger, never stretched;
 * - where there is no picture either (a host that takes none), an empty pane: its colour and the faint mark.
 *
 * Nothing a pointer or a screen reader meets: the drag announces itself.
 */

import { Show, createEffect, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { EmptyPaneLogo } from "@/app/element/EmptyPaneLogo";
import { landingPictureOf, type LandingPicture } from "./paneLandingPicture";
import { buildTabShell, showTabLanding, tabMemberLands, tabShellOf } from "./tabLanding";

/** The tab's own content in the shell it had in its pane. */
const TabLanding = (props: { blockId: string }): JSX.Element => {
    const { root, content } = buildTabShell(tabShellOf(props.blockId));
    onMount(() => onCleanup(showTabLanding(props.blockId, content)));
    return root;
};

/** The tab's picture at its own size, or the empty pane where there is none. */
const PictureLanding = (props: { blockId: string }): JSX.Element => {
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
        <Show when={shot()} fallback={<EmptyPaneLogo />}>
            {(got) => (
                <img
                    src={`data:image/jpeg;base64,${got().picture}`}
                    alt=""
                    draggable={false}
                    style={{ width: `${got().size.width}px`, height: `${got().size.height}px` }}
                />
            )}
        </Show>
    );
};

export const PaneLanding = (props: { blockId: string }): JSX.Element => {
    // Read once: the place is built for the tab as it stands when the place is first shown.
    const real = tabMemberLands(props.blockId);
    return (
        <div class="pane-landing" aria-hidden="true" ref={(el) => (el.inert = true)}>
            <div class="pane-landing-pane" classList={{ "pane-landing-pane--picture": !real }}>
                {real ? <TabLanding blockId={props.blockId} /> : <PictureLanding blockId={props.blockId} />}
            </div>
        </div>
    );
};
