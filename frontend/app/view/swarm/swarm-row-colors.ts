// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The two border colours a Swarm agent card shows, both derived from the agent's
 * own pane so they always match what that pane's chrome shows.
 * SPEC_COMPOSER_ACCOUNT_SWITCH_AND_JEKT_HEIGHT_CAP_2026_09_26.md Part C.
 *
 * - `active`: the selected card's border, the pane tab's selected colour
 *   (`computeBlockActiveBorderColor`: `frame:hue` wins, else
 *   `frame:activebordercolor`).
 * - `hover`: the hovered card's border, the colour an UNSELECTED pane's border
 *   shows (`computeFocusRingBorderColor(false, ...)`: `frame:hue` wins, else
 *   `frame:bordercolor`, the dimmed counterpart). Hover and selection therefore
 *   draw the same outline and differ only in shade.
 *
 * Either is `undefined` when the block has no colour; the stylesheet's
 * `var(--x, <default>)` fallbacks take over.
 */

import { computeBlockActiveBorderColor, computeFocusRingBorderColor } from "@/app/block/blockframe";

export interface SwarmRowColors {
    active: string | undefined;
    hover: string | undefined;
}

export function swarmRowColors(blockMeta: Block["meta"] | undefined): SwarmRowColors {
    return {
        active: computeBlockActiveBorderColor(blockMeta),
        hover: computeFocusRingBorderColor(false, blockMeta),
    };
}
