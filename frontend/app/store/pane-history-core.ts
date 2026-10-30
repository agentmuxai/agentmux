// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane selection history, as pure functions: a list of the panes the user
// selected, across window tabs, and a cursor, like a browser's. Back and
// Forward move the cursor; a new selection after going Back drops what was
// ahead. SPEC_MOUSE_SIDE_BUTTONS_PANE_AND_TAB_NAV_2026_10_19.md §4.1.

export interface PaneRef {
    tabId: string;
    blockId: string;
}

export interface PaneHistory {
    entries: PaneRef[];
    /** The entry the user is on; -1 when empty. */
    index: number;
}

export const PANE_HISTORY_CAP = 100;

export function emptyHistory(): PaneHistory {
    return { entries: [], index: -1 };
}

export function samePane(a: PaneRef | null | undefined, b: PaneRef | null | undefined): boolean {
    return a != null && b != null && a.tabId === b.tabId && a.blockId === b.blockId;
}

/** The user selected `ref`. Selecting the pane already current adds nothing. */
export function recordPane(h: PaneHistory, ref: PaneRef): PaneHistory {
    if (samePane(h.entries[h.index], ref)) return h;
    let entries = [...h.entries.slice(0, h.index + 1), { ...ref }];
    if (entries.length > PANE_HISTORY_CAP) entries = entries.slice(entries.length - PANE_HISTORY_CAP);
    return { entries, index: entries.length - 1 };
}

/** A tab of the window and the blocks it holds; `blockIds` is null when the tab isn't loaded. */
interface TabBlocks {
    tabId: string;
    blockIds: string[] | null;
}

/**
 * Where the pane `ref` is now: `ref` itself when its tab still holds the block (or can't be checked),
 * the tab that holds the block when it was moved to another, null when it is nowhere in `tabs`.
 */
export function locatePane(ref: PaneRef, tabs: TabBlocks[]): PaneRef | null {
    const own = tabs.find((t) => t.tabId === ref.tabId);
    if (own && (own.blockIds == null || own.blockIds.includes(ref.blockId))) return ref;
    const holder = tabs.find((t) => t.tabId !== ref.tabId && t.blockIds?.includes(ref.blockId));
    return holder ? { tabId: holder.tabId, blockId: ref.blockId } : null;
}

export interface StepResult {
    /** `h` without the dead entries passed on the way, and the entry to go to re-pointed if its pane
     *  was moved to another tab; the cursor still on the current entry. */
    history: PaneHistory;
    /** The entry to go to (an entry of `history`), or null when nothing live lies that way. */
    target: PaneRef | null;
}

/**
 * The next live entry from the cursor in `dir` (-1 Back, +1 Forward). `locate` says where an entry's
 * pane is now: the same place, another tab (the entry follows it), or null (gone: the entry is skipped
 * and removed). Doesn't move the cursor: `moveTo` does, once the pane is selected.
 */
export function stepHistory(h: PaneHistory, dir: -1 | 1, locate: (ref: PaneRef) => PaneRef | null): StepResult {
    const current = h.entries[h.index];
    const dead = new Set<PaneRef>();
    let j = h.index + dir;
    let target: PaneRef | null = null;
    let moved: { from: PaneRef; to: PaneRef } | null = null;
    while (j >= 0 && j < h.entries.length) {
        const entry = h.entries[j];
        const where = locate(entry);
        if (where) {
            target = samePane(where, entry) ? entry : { ...where };
            if (target !== entry) moved = { from: entry, to: target };
            break;
        }
        dead.add(entry);
        j += dir;
    }
    if (dead.size === 0 && !moved) return { history: h, target };
    const entries = h.entries.filter((e) => !dead.has(e)).map((e) => (e === moved?.from ? moved.to : e));
    return { history: { entries, index: current ? entries.indexOf(current) : -1 }, target };
}

/** The cursor moves to `ref`, an entry of `h`. */
export function moveTo(h: PaneHistory, ref: PaneRef): PaneHistory {
    const index = h.entries.indexOf(ref);
    return index < 0 ? h : { entries: h.entries, index };
}

/** `ref` can't be selected after all: forget it. */
export function dropEntry(h: PaneHistory, ref: PaneRef): PaneHistory {
    const i = h.entries.indexOf(ref);
    if (i < 0) return h;
    const entries = h.entries.filter((_, k) => k !== i);
    return { entries, index: i < h.index ? h.index - 1 : h.index };
}
