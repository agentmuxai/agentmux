// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Two document panes (Editors) that end up as tabs of one pane become one
 * (SPEC_DOCUMENT_PANES_MERGE_DOCUMENT_TABS_2026_10_30.md). The layout may not import the Editor, so
 * the app registers what knows about documents, as `setMinimizeMerger` does: `merge` takes the
 * arriving blocks' documents into the stack's destination and answers the blocks that have nothing
 * left, `mergeable` says which members of a stack could be merged (the first, then the rest).
 *
 * Its own module with no imports, so layout/index.ts can register it while layoutStack.ts is still
 * being loaded (the two import each other through the app's stores).
 */
export interface DocPaneMerger {
    merge(targetBlockId: string, stack: readonly string[], arriving: readonly string[]): Promise<string[]>;
    mergeable(stack: readonly string[]): string[];
}

let docPaneMerger: DocPaneMerger | undefined;

export function setDocPaneMerger(merger: DocPaneMerger | undefined): void {
    docPaneMerger = merger;
}

export function getDocPaneMerger(): DocPaneMerger | undefined {
    return docPaneMerger;
}
