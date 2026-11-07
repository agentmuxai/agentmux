// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * GripDots — a 4×3 dot grip (two 2×3 grips side by side) marking a drag
 * space: a pane's grip, and with `rows={4}` the window's (4×4). Font Awesome
 * Free has 3×2 and 2×3 grips but no 4×3, so it is drawn here. It takes its
 * colour from `currentColor` and its size from the font size, like an icon
 * glyph: 3 rows are 1em tall, 4 rows 1.333em, so the dots keep one size and
 * one 4-unit pitch.
 *
 * See docs/specs/SPEC_DRAG_SPACE_AFFORDANCE_2026_10_13.md.
 */

import type { JSX } from "solid-js";

const COLS = [2, 6, 10, 14];

export const GripDots = (props: { class?: string; rows?: 3 | 4 }): JSX.Element => {
    const rows = () => props.rows ?? 3;
    return (
        <svg
            class={props.class}
            width="1.333em"
            height={rows() === 4 ? "1.333em" : "1em"}
            viewBox={rows() === 4 ? "0 0 16 16" : "0 0 16 12"}
            fill="currentColor"
            aria-hidden="true"
        >
            {Array.from({ length: rows() }, (_, r) => r * 4 + 2).flatMap((cy) =>
                COLS.map((cx) => <circle cx={cx} cy={cy} r="1.25" />)
            )}
        </svg>
    );
};
