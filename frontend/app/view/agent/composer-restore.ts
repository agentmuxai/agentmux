// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Put text back into a pane's composer from outside it, after the pane took the text off the
 * conversation and then could not use it (a refused send). The composer's textarea is uncontrolled
 * and the footer unmounts on a tab switch, so there are two ways in: the mounted footer registers a
 * writer, and with none the text goes into the stored draft, which the footer reads when it mounts.
 */

import { getComposerDraft, setComposerDraft } from "./reload-persistence";

/** What is already in the composer, then the restored text after a blank line. */
export function joinDraft(current: string, restored: string): string {
    return current.trim() ? `${current.replace(/\s+$/, "")}\n\n${restored}` : restored;
}

const writers = new Map<string, (restored: string) => void>();

/** The mounted footer's way to add `restored` to what it holds; returns what unregisters it. */
export function registerComposerRestore(blockId: string, write: (restored: string) => void): () => void {
    writers.set(blockId, write);
    return () => {
        if (writers.get(blockId) === write) writers.delete(blockId);
    };
}

export function restoreToComposer(blockId: string, restored: string): void {
    if (!restored) return;
    const write = writers.get(blockId);
    if (write) write(restored);
    else setComposerDraft(blockId, joinDraft(getComposerDraft(blockId) ?? "", restored));
}
