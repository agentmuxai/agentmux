// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a pane's rect is laid out against, in physical pixels: the page's size and the rect of the tile
 * layout holding the pane. While a window is being resized the host stretches a pane's last rect by how
 * much the window has changed since (browser_panes/bounds.rs `LayoutBase`), so the pane moves with the
 * window instead of a frame or two after the page.
 *
 * Take it when the rect is measured and send it with that rect. A busy page sends a rect some frames after
 * it measured it; a base read at send time then describes a window newer than the rect, the host takes the
 * rect as current, and the pane stays at the old layout (task #47927).
 */
export function paneLayoutBase(blockId: string | undefined): PaneLayoutBase {
    const dpr = window.devicePixelRatio || 1;
    const px = (v: number) => Math.round(v * dpr);
    const tiles = blockId ? document.querySelector(`[data-blockid="${blockId}"]`)?.closest(".tile-layout") : null;
    const r = tiles?.getBoundingClientRect();
    return {
        viewport: { width: px(window.innerWidth), height: px(window.innerHeight) },
        ...(r && r.width > 0 && r.height > 0
            ? { layout: { x: px(r.x), y: px(r.y), width: px(r.width), height: px(r.height) } }
            : {}),
    };
}
