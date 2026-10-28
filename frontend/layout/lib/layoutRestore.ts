// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { isEffectivelyMinimized } from "./layoutInvariants";
import { findParent, newLayoutNode } from "./layoutNode";
import { resolveRowSlipTargets } from "./layoutSlip";
import { DefaultNodeSize, FlexDirection, type LayoutNode } from "./types";

/**
 * # A restored chip grows in the column it shows in
 *
 * A minimized pane in a Row has no column of its own: its header chip docks onto the nearest
 * expanded sibling and stacks above it (`resolveRowSlipTargets`, `updateTreeHelper`). Clearing the
 * flag alone would hand the pane its old Row slot back, a new column beside the one the chip was
 * shown in. The pane is expanded where its chip is instead: before the flag is cleared, the chips
 * docked onto the same pane move into that pane's column, in the order they were stacked, so they
 * are chips in a column (which render in place) and the restored one grows vertically in its own
 * spot, keeping the column's width. The other panes in the column give up height by the usual
 * rules. srv does the same (`restore_node`, crates/srv/src/backend/layout/intent.rs).
 */

/** The chips a restore of `node` takes along, in stacking order, and the pane they dock onto.
 *  Undefined when the pane is not a docked chip: it is in a Column (its chip shows in place), or no
 *  sibling of its Row is expanded. */
export function dockedStackOf(
    root: LayoutNode,
    node: LayoutNode
): { row: LayoutNode; dock: LayoutNode; items: LayoutNode[] } | undefined {
    if (node.children?.length || !node.minimized) return undefined;
    let top = node;
    let parent = findParent(root, top.id);
    while (parent && isEffectivelyMinimized(parent)) {
        top = parent;
        parent = findParent(root, top.id);
    }
    if (!parent || parent.flexDirection !== FlexDirection.Row) return undefined;
    const targets = resolveRowSlipTargets(parent.children);
    const dock = targets.get(top.id);
    if (!dock) return undefined;
    return { row: parent, dock, items: parent.children.filter((c) => targets.get(c.id)?.id === dock.id) };
}

/** Move the docked chips (`items`, gathered before the flag was cleared) into `dock`'s column. */
export function moveStackIntoDockColumn(row: LayoutNode, dock: LayoutNode, items: LayoutNode[]) {
    const column = dock.children?.length && dock.flexDirection === FlexDirection.Column;
    // Each moved pane gets a share the column's own panes have, in the column's units.
    const sibs = column ? dock.children.filter((c) => c.size > 0) : [];
    const share = column
        ? sibs.length
            ? sibs.reduce((s, c) => s + c.size, 0) / sibs.length
            : DefaultNodeSize
        : dock.size;
    // A stack of chips that is itself a column gives its panes to the column.
    const moved = items.flatMap((item) =>
        item.children?.length && item.flexDirection === FlexDirection.Column ? item.children : [item]
    );
    moved.forEach((leaf) => {
        if (!leaf.children?.length) leaf.size = share;
    });
    row.children = row.children.filter((c) => !items.includes(c));
    if (column) {
        dock.children = [...moved, ...dock.children];
        return;
    }
    const group = newLayoutNode(FlexDirection.Column, dock.size, [dock]);
    group.children = [...moved, dock];
    row.children = row.children.map((c) => (c === dock ? group : c));
}
