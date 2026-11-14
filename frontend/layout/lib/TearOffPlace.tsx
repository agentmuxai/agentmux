// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The dragged pane's own place at the start of its drag, behind the pane in hand: a release there tears the
 * pane off, and the place says so. A red and light red checkerboard with the AgentMux mark over it, a bit
 * transparent, and the words on top (the operator, 2026-11-13: "it should be red/light red checkerboard with
 * the logo a bit transparent in the background"); nothing of the pane shows through. Drawn by the tile while
 * `tearOffOffered` holds (futureLayout.ts), styled in tilelayout.scss. Nothing a pointer or a screen reader
 * meets: the drag announces itself.
 *
 * One tab dragged out of a pane that keeps others has the same place, under the pane's header: the pane and
 * its other tabs stay, and only the tab's own place says it tears off (tabTearOff.ts). `top` is where that
 * place starts, from the tile's top. Once the tab has left the pane for the rest of its drag there is no
 * place: the pane shows the tab that comes to the front when the dragged one goes
 * (app/tab/pane-leaf-state.ts `useShownBlockId`).
 */

import type { JSX } from "solid-js";
import { EmptyPaneLogo } from "@/app/element/EmptyPaneLogo";

const under = (top: number | undefined): JSX.CSSProperties | undefined => (top ? { top: `${top}px` } : undefined);

export const TearOffPlace = (props: { top?: number }): JSX.Element => (
    <div class="tear-off-place" aria-hidden="true" style={under(props.top)}>
        <EmptyPaneLogo />
        <span class="tear-off-words">TEAR OFF</span>
    </div>
);
