// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The dragged pane's own place at the start of its drag, behind the pane in hand: a release there tears the
 * pane off, and the place says so. A red and light red checkerboard with the AgentMux mark over it, a bit
 * transparent, and the words on top (the operator, 2026-11-13: "it should be red/light red checkerboard with
 * the logo a bit transparent in the background"); nothing of the pane shows through. Drawn by the tile while
 * `tearOffOffered` holds (futureLayout.ts), styled in tilelayout.scss. Nothing a pointer or a screen reader
 * meets: the drag announces itself.
 */

import type { JSX } from "solid-js";
import { EmptyPaneLogo } from "@/app/element/EmptyPaneLogo";

export const TearOffPlace = (): JSX.Element => (
    <div class="tear-off-place" aria-hidden="true">
        <EmptyPaneLogo />
        <span class="tear-off-words">TEAR OFF</span>
    </div>
);
