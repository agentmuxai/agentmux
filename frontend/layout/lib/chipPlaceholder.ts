// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { FlexDirection } from "./types";

/**
 * Where the drop placeholder goes when the pane being dragged is a minimized pane (a chip).
 *
 * It has the chip's own size (SPEC_DRAG_MINIMIZED_PANES_2026_10_20.md §3 A) and the expanded
 * placeholder's position: between two panes it is centred on the seam it will be inserted at,
 * half over each, and at the first or last place it is flush inside the edge
 * (SPEC_CHIP_DROP_PLACEHOLDER_ON_THE_SEAM_2026_10_21.md).
 *
 * `target` is the pane the drop is placed against, `after` whether it goes after that pane, and
 * `outerEdge` whether there is no pane on the other side of the seam.
 */
export function chipPlaceholderRect(
    target: Dimensions,
    chip: { width: number; height: number },
    direction: FlexDirection,
    after: boolean,
    outerEdge: boolean
): Dimensions {
    const along = (start: number, length: number, size: number) => {
        const seam = after ? start + length : start;
        if (!outerEdge) return seam - size / 2;
        return after ? seam - size : seam;
    };
    const height = Math.min(chip.height, target.height);
    if (direction === FlexDirection.Column) {
        return { left: target.left, width: target.width, height, top: along(target.top, target.height, height) };
    }
    const width = Math.min(chip.width, target.width);
    return { top: target.top, height, width, left: along(target.left, target.width, width) };
}
