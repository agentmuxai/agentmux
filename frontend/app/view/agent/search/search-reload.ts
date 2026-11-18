// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What an open find bar keeps across a window recycle: that it is open, what
 * was typed and which match was current. A search bar nobody is typing in is
 * not a reason to keep a window at several gigabytes (task #47824: it declined
 * 77 of 79 recycle requests in one day), so the recycle writes it down and the
 * reloaded pane puts it back, without taking the caret or moving the view.
 * Stored in sessionStorage like the composer's drafts (reload-persistence.ts):
 * it survives a reload of this window, not an app restart.
 */

import { registerReloadSaver } from "@/app/store/recycle-savers";
import { readSession, writeSession } from "../reload-persistence";

const SEARCH_KEY = "agentmux:find-in-pane:";

export interface StoredSearch {
    /** What was typed, as typed. Empty for an open bar with nothing in it. */
    text: string;
    /** The node of the current match, or null when none was current. */
    matchId: string | null;
    /** Whether rows that do not match are dimmed. */
    dim: boolean;
}

/** Each mounted pane's read of its search: the state to keep, or null while the bar is closed. */
const live = new Map<string, () => StoredSearch | null>();

/** A pane says how to read its search; the returned function removes it. */
export function registerSearchForReload(blockId: string, read: () => StoredSearch | null): () => void {
    live.set(blockId, read);
    return () => {
        if (live.get(blockId) === read) live.delete(blockId);
    };
}

/** What a recycle kept for this pane's find bar, once: the first read takes it out of storage. */
export function takeStoredSearch(blockId: string): StoredSearch | undefined {
    const stored = readSession<Partial<StoredSearch>>(SEARCH_KEY + blockId);
    if (stored === undefined) return undefined;
    writeSession(SEARCH_KEY + blockId, undefined);
    if (typeof stored.text !== "string") return undefined;
    return { text: stored.text, matchId: typeof stored.matchId === "string" ? stored.matchId : null, dim: stored.dim !== false };
}

// A bar that couldn't be stored just doesn't come back: never a reason to keep the window as it is.
registerReloadSaver(() => {
    for (const [blockId, read] of live) writeSession(SEARCH_KEY + blockId, read() ?? undefined);
    return null;
});
