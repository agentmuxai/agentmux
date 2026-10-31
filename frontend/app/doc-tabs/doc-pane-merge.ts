// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Two document panes (Editors) that end up as tabs of one pane become one: the arriving pane's
 * document tabs join the destination's, and the arriving pane tab, with nothing left in it, goes.
 * The layout calls `mergeArrivingDocumentPanes` after a person's move into a stack
 * (layoutStack.ts `setDocPaneMerger`); the pane-tab menu calls `mergeDocumentPanesInStack` for
 * the stacks a move never reached (a saved layout, an agent's move).
 * docs/specs/SPEC_DOCUMENT_PANES_MERGE_DOCUMENT_TABS_2026_10_30.md.
 */

import { notifyDrop } from "@/app/drag/file-drop-actions";
import { docTabHost, whenDocTabHost, type DocTabHost } from "./doc-tab-hosts";

/** How long a pane that has just arrived gets to mount and register its host. */
const HOST_WAIT_MS = 1500;

/** Whether `into` can take `from`'s documents: it merges, and the two are the same type of pane
 *  on the same computer (a path means nothing on another). */
export function canMergeDocPanes(into: DocTabHost | undefined, from: DocTabHost | undefined): boolean {
    if (!into?.mergeFrom || !from || into === from) return false;
    return into.docType === from.docType && (into.scope?.() ?? "") === (from.scope?.() ?? "");
}

/**
 * Merge each of `arriving` (blocks that have just joined a stack) into the stack's destination
 * document pane: `targetBlockId` when it is one, else the first other member that is. Returns the
 * blocks whose documents are all in the destination now, in order: the caller closes them. A
 * pane that cannot merge (another type or computer, a save in flight) is left as it is; a
 * refusal that is worth saying is said.
 */
export async function mergeArrivingDocumentPanes(
    targetBlockId: string,
    stack: readonly string[],
    arriving: readonly string[]
): Promise<string[]> {
    const merged: string[] = [];
    for (const [at, id] of arriving.entries()) {
        const source = await whenDocTabHost(id, HOST_WAIT_MS);
        if (!source) continue;
        // A pane that came in with it and has not been looked at yet is not a destination; one that
        // was looked at and stayed (the first of two document panes dropped together) is.
        const notYet = new Set(arriving.slice(at + 1));
        const destination = [targetBlockId, ...stack]
            .filter((b) => b !== id && !notYet.has(b) && !merged.includes(b))
            .map((b) => docTabHost(b))
            .find((h) => canMergeDocPanes(h, source));
        if (!destination) continue;
        const result = await destination.mergeFrom!(source);
        if (result.merged) merged.push(id);
        else if (result.merged === false && result.reason) notifyDrop.cantMove("its documents", result.reason);
    }
    return merged;
}

/** The members of `stack` that hold documents and could be merged into the first of their kind:
 *  every later member the first can take. Empty when there is nothing to merge. */
export function mergeableDocumentPanes(stack: readonly string[]): string[] {
    for (const id of stack) {
        const into = docTabHost(id);
        if (!into?.mergeFrom) continue;
        const others = stack.filter((b) => b !== id && canMergeDocPanes(into, docTabHost(b)));
        if (others.length) return [id, ...others];
    }
    return [];
}
