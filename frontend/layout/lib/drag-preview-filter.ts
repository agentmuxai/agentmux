// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which parts of a pane the drag ghost's picture clones. The ghost is the pane drawn off screen in a box of the
 * preview's size (a few hundred pixels at most), so what lies outside that box never shows. `toPng` clones and restyles
 * every element it is given in one synchronous run, which for a pane with a long transcript or a large tree took 176 to
 * 317 ms of the main thread inside whatever click, drop or drag was under way. An element wholly outside the box is left
 * out, with everything under it; the picture is the same.
 */

/** Slack, in pixels, so an element that touches the box's edge is kept. */
const EDGE_SLACK = 1;

/** The `filter` for `toPng` of `root`: false for an element that lies wholly outside `root`'s box. */
export function insideGhost(root: HTMLElement): (node: HTMLElement) => boolean {
    const box = root.getBoundingClientRect();
    // A box with no size is not measured (the preview not laid out): nothing can be told to be outside it.
    if (box.width === 0 || box.height === 0) return () => true;
    return (node) => {
        if (node === root || typeof node.getBoundingClientRect !== "function") return true;
        const r = node.getBoundingClientRect();
        // An element with no box of its own (display: contents, or not rendered) may still hold elements that have one.
        if (r.width === 0 && r.height === 0) return true;
        return (
            r.bottom > box.top - EDGE_SLACK &&
            r.top < box.bottom + EDGE_SLACK &&
            r.right > box.left - EDGE_SLACK &&
            r.left < box.right + EDGE_SLACK
        );
    };
}
