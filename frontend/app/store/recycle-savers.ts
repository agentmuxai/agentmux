// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What a window's renderer recycle writes down before it reloads the page:
 * each feature that keeps state for a reload registers a save, so the recycle
 * (recycle.ts) needn't know the agent view's modules
 * (docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §3). A save
 * returns why it could not store something, in words for the user, or null.
 * A module that was never loaded holds nothing, so it has nothing to save.
 */

type ReloadSaver = () => string | null | Promise<string | null>;

const savers = new Set<ReloadSaver>();

/** Add a save; the returned function removes it. */
export function registerReloadSaver(save: ReloadSaver): () => void {
    savers.add(save);
    return () => {
        savers.delete(save);
    };
}

/** Run every save, one after another; the reasons for the ones that could not store everything. */
export async function saveForReload(): Promise<string[]> {
    const unsaved: string[] = [];
    for (const save of [...savers]) {
        const reason = await save();
        if (reason) unsaved.push(reason);
    }
    return unsaved;
}
