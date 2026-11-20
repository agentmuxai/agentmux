// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The comparison layout-stress.mjs makes after each step: what the window draws against the layout srv stored.
// Pure functions, so they are tested without a window (layout-stress-check.test.mjs).

export function leavesOf(node, out = []) {
    if (!node) return out;
    if (node.children?.length) node.children.forEach((c) => leavesOf(c, out));
    else if (node.data) out.push(node);
    return out;
}

/** The disagreements between what is drawn (`dom`) and what is stored (`layout`): a list of sentences. */
export function disagreements(dom, layout) {
    const out = [];
    const ids = dom.tiles.map((t) => t.id);
    if (new Set(ids).size !== ids.length) out.push("a pane is drawn twice");
    const stored = new Map(layout.leaves.map((l) => [l.id, l]));
    for (const t of dom.tiles) if (!stored.has(t.id)) out.push(`pane ${t.id.slice(0, 8)} is drawn but not stored`);
    for (const l of layout.leaves) if (!ids.includes(l.id)) out.push(`pane ${l.id.slice(0, 8)} is stored but not drawn`);
    for (const t of dom.tiles) {
        const l = stored.get(t.id);
        if (l && Boolean(l.minimized) !== t.minimized) out.push(`pane ${t.id.slice(0, 8)} is ${t.minimized ? "drawn minimized, stored expanded" : "drawn expanded, stored minimized"}`);
    }
    const maxLeaf = layout.magnifiednodeid ? stored.get(layout.magnifiednodeid) : undefined;
    if (layout.magnifiednodeid && !maxLeaf) out.push("the stored maximized pane is not a leaf");
    if (Boolean(layout.magnifiednodeid) !== Boolean(dom.maximizedBlockId)) out.push(layout.magnifiednodeid ? "a pane is stored maximized and none is drawn so" : "a pane is drawn maximized and none is stored so");
    else if (maxLeaf && dom.maximizedBlockId && maxLeaf.data.blockId !== dom.maximizedBlockId) out.push("the maximized pane drawn is not the one stored");
    const expandedStored = layout.leaves.filter((l) => !l.minimized).length;
    const expandedDrawn = dom.tiles.filter((t) => !t.minimized).length;
    if (layout.leaves.length > 0 && expandedStored < 1) out.push("every stored pane is minimized");
    if (dom.tiles.length > 0 && expandedDrawn < 1 && !dom.maximizedBlockId) out.push("every pane drawn is minimized");
    if (layout.focusednodeid && !stored.has(layout.focusednodeid)) out.push("the stored focused pane is not a leaf");
    if (!dom.maximizedBlockId) out.push(...geometry(dom));
    return out;
}

/** Expanded panes have a size, stay in the layout area and do not overlap. */
function geometry(dom) {
    const out = [];
    const open = dom.tiles.filter((t) => !t.minimized);
    const [ax, ay, aw, ah] = dom.area ?? [0, 0, Infinity, Infinity];
    for (const t of open) {
        const [x, y, w, h] = t.rect;
        if (w < 20 || h < 20) out.push(`pane ${t.id.slice(0, 8)} is ${Math.round(w)}x${Math.round(h)}`);
        if (x < ax - 2 || y < ay - 2 || x + w > ax + aw + 2 || y + h > ay + ah + 2) out.push(`pane ${t.id.slice(0, 8)} leaves the layout area`);
    }
    for (let i = 0; i < open.length; i++) {
        for (let j = i + 1; j < open.length; j++) {
            const [x1, y1, w1, h1] = open[i].rect;
            const [x2, y2, w2, h2] = open[j].rect;
            const ox = Math.min(x1 + w1, x2 + w2) - Math.max(x1, x2);
            const oy = Math.min(y1 + h1, y2 + h2) - Math.max(y1, y2);
            if (ox > 2 && oy > 2) out.push(`panes ${open[i].id.slice(0, 8)} and ${open[j].id.slice(0, 8)} overlap by ${Math.round(ox)}x${Math.round(oy)}`);
        }
    }
    return out;
}
