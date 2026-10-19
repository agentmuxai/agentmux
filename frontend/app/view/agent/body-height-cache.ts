// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The heights of open tool bodies, read together before their results unload.
 *
 * A tool row whose result unloads keeps its height (a placeholder as tall as
 * the body it replaces, so the rows below don't move). It used to read that
 * height as each row's own body was torn down, in the middle of the update that
 * unloads them all: every read came after the previous row's DOM change, so each
 * forced a layout of the pane, one per open body, ~175 ms per agent open in the
 * lab (CPU profile of the open; ToolBlock's `offsetHeight` was the largest
 * forced-layout caller). The pass that plans the unload reads every height first,
 * with no write between, which costs one layout, and each row takes its own
 * height from here.
 */

import { planUnload } from "./tool-result-unload";
import type { DocumentNode } from "./types";

const remembered = new Map<string, number>();

/** `ids`' open bodies under `root`, measured in one read pass. */
export function rememberBodyHeights(root: ParentNode | null, ids: readonly string[]): void {
    if (!root || ids.length === 0) return;
    const wanted = new Set(ids);
    for (const body of root.querySelectorAll<HTMLElement>(".agent-tool-overlay")) {
        const id = body.closest<HTMLElement>("[data-node-id]")?.dataset.nodeId;
        if (id && wanted.has(id)) remembered.set(id, body.offsetHeight); // perf:allow-layout-read — every height in one pass, before any write
    }
}

/** The height remembered for `nodeId`, once: a row takes it as its result unloads. */
export function takeBodyHeight(nodeId: string): number | undefined {
    const px = remembered.get(nodeId);
    remembered.delete(nodeId);
    return px;
}

/** Drop what was remembered for `ids` and not taken (their results did not unload). */
export function forgetBodyHeights(ids: readonly string[]): void {
    for (const id of ids) remembered.delete(id);
}

/**
 * Run `unload` (the document change that unloads the results `planUnload` picks
 * from `nodes`) with the open bodies' heights read first.
 */
export function withBodyHeights<T>(
    root: ParentNode | null,
    nodes: readonly DocumentNode[],
    keepIds: ReadonlySet<string>,
    unload: () => T
): T {
    const ids = root ? planUnload(nodes, { keepIds }) : [];
    rememberBodyHeights(root, ids);
    try {
        return unload();
    } finally {
        forgetBodyHeights(ids);
    }
}
