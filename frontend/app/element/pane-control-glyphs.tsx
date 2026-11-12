// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Glyphs for the pane header's controls that Font Awesome Free has no icon for.
 * SPEC_PANE_CONTROLS_TILED_RESTORE_ICON_AND_MINIMIZE_FROM_MAXIMIZED_2026_10_12.md §4.
 *
 * Passed as an `IconButtonDecl.icon` (IconButton renders a non-string icon as
 * is). They draw with `currentColor`, so the button's colour and hover rules
 * apply, and fill an 18px box (1em at the header's 18px font-size) so they sit
 * on the same line as the font icons beside them.
 */

import { type JSX } from "solid-js";

/**
 * Restore, for a tiled layout: a frame split into tiles with this pane's tile
 * filled. AgentMux panes never overlap, so the two-overlapping-windows
 * `window-restore` was wrong for "put this pane back in its tile"; it stays
 * where a real second window is meant (New Window, browser pop-out).
 *
 * Ink is 17.7 × 15.7px, centred (y 1.15 → 16.85): the same extent as
 * `regular@window-maximize` beside it. The stroke is 1.36px (80% of Free
 * regular's ~1.7px), so the frame is 0.85px in from the box edge.
 */
export const PaneRestoreGlyph = (): JSX.Element => (
    <svg
        class="pane-control-glyph pane-restore-glyph"
        viewBox="0 0 18 18"
        width="1em"
        height="1em"
        fill="none"
        stroke="currentColor"
        stroke-width="1.36"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
        data-testid="pane-restore-glyph"
    >
        <rect x="0.85" y="1.83" width="16.3" height="14.34" rx="2" />
        <path d="M10.5 1.83L10.5 16.17" />
        <path d="M10.5 9L17.15 9" />
        <rect x="1.93" y="2.91" width="7.5" height="12.18" rx="0.75" fill="currentColor" stroke="none" />
    </svg>
);

/**
 * The two stacked minimize buttons' glyphs (SPEC_PANE_HEADER_TWO_MINIMIZE_BUTTONS_2026_11_08.md §4).
 * Both are 16 x 2 px bars in one box, so the pair reads as two evenly spaced rules: "Minimize to
 * header" is one solid bar, "Merge into next pane" the same bar with two 2 px gaps cut in (three 4 px
 * segments at whole pixels, so the rhythm stays crisp and does not blur into a dashed haze).
 */
const BAR_BOX = { viewBox: "0 0 16 2", width: 16, height: 2 } as const;

export const PaneMinimizeGlyph = (): JSX.Element => (
    <svg
        class="pane-control-glyph pane-minimize-glyph"
        {...BAR_BOX}
        fill="currentColor"
        shape-rendering="crispEdges"
        aria-hidden="true"
        data-testid="pane-minimize-glyph"
    >
        <rect x="0" y="0" width="16" height="2" />
    </svg>
);

export const PaneMergeGlyph = (): JSX.Element => (
    <svg
        class="pane-control-glyph pane-merge-glyph"
        {...BAR_BOX}
        fill="currentColor"
        shape-rendering="crispEdges"
        aria-hidden="true"
        data-testid="pane-merge-glyph"
    >
        <rect x="0" y="0" width="4" height="2" />
        <rect x="6" y="0" width="4" height="2" />
        <rect x="12" y="0" width="4" height="2" />
    </svg>
);
