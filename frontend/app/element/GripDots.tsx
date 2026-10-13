// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * GripDots — a 4×3 dot grip (two 2×3 grips side by side) marking a drag
 * space: the window's drag square, a pane's grip. Font Awesome Free has 3×2
 * and 2×3 grips but no 4×3, so it is drawn here. It takes its colour from
 * `currentColor` and its height from the font size (1em), like an icon glyph.
 *
 * See docs/specs/SPEC_DRAG_SPACE_AFFORDANCE_2026_10_13.md.
 */

import type { JSX } from "solid-js";

const COLS = [2, 6, 10, 14];
const ROWS = [2, 6, 10];

export const GripDots = (props: { class?: string }): JSX.Element => (
    <svg
        class={props.class}
        width="1.333em"
        height="1em"
        viewBox="0 0 16 12"
        fill="currentColor"
        aria-hidden="true"
    >
        {ROWS.flatMap((cy) => COLS.map((cx) => <circle cx={cx} cy={cy} r="1.25" />))}
    </svg>
);
