// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Whether an Editor's file tree starts open. An Editor opened on a file starts with it collapsed, whoever
 * opened it (the Files pane, a link, an agent, the palette), so a new way of opening files needs no say;
 * one with no file keeps the tree, to pick one from. A value the pane already holds wins (the person's
 * toggle, an explicit caller), and the model writes the first decision back, so a pane that began without
 * a file stays open when a file is opened in it later.
 */

export const META_TREE_EXPANDED = "editor:tree_expanded";

export function initialTreeExpanded(meta: Record<string, unknown> | undefined): boolean {
    const held = meta?.[META_TREE_EXPANDED];
    if (typeof held === "boolean") return held;
    const file = meta?.file;
    return !(typeof file === "string" && file !== "");
}
