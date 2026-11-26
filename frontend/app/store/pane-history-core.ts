// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The window's trail of places, as pure functions: a list of the places the user went to, across window
// tabs, and a cursor, like a browser's. A place is a pane and, where its view has joined, where the view
// stands inside itself. Back and Forward move the cursor; a new place after going Back drops what was
// ahead. SPEC_MOUSE_SIDE_BUTTONS_PANE_AND_TAB_NAV_2026_10_19.md §4.1 and
// SPEC_BACK_FORWARD_FOLLOW_THE_WHOLE_TRAIL_2026_11_26.md §3.

/** Where a view stands inside its pane: plain data (ids, paths, a section), never content. */
export type ViewPlace = Record<string, unknown>;

export interface PaneRef {
    tabId: string;
    blockId: string;
    /** Where the pane's view stood; absent for a view that has not joined (the entry is pane-level). */
    view?: ViewPlace;
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

/** `value` as a string that is the same for equal data, whatever order its keys were written in. */
function canonical(value: unknown): string {
    if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
    if (value && typeof value === "object") {
        const o = value as Record<string, unknown>;
        return `{${Object.keys(o)
            .filter((k) => o[k] !== undefined)
            .sort()
            .map((k) => `${JSON.stringify(k)}:${canonical(o[k])}`)
            .join(",")}}`;
    }
    return JSON.stringify(value) ?? "null";
}

/** Whether two view places are the same: none and an empty one are the same. */
export function sameView(a: ViewPlace | undefined, b: ViewPlace | undefined): boolean {
    return canonical(a ?? {}) === canonical(b ?? {});
}

/** The same pane, and the same place in its view. */
export function samePlace(a: PaneRef | null | undefined, b: PaneRef | null | undefined): boolean {
    return samePane(a, b) && sameView(a?.view, b?.view);
}

function capped(entries: PaneRef[]): PaneRef[] {
    return entries.length > PANE_HISTORY_CAP ? entries.slice(entries.length - PANE_HISTORY_CAP) : entries;
}

/** The user went to `ref`. Going to the place already current adds nothing. */
export function recordPane(h: PaneHistory, ref: PaneRef): PaneHistory {
    if (samePlace(h.entries[h.index], ref)) return h;
    const entries = capped([...h.entries.slice(0, h.index + 1), { ...ref }]);
    return { entries, index: entries.length - 1 };
}

/**
 * The current entry becomes `ref`: what the same gesture settled on after it first pushed (a redirect, or
 * pushes in one frame). An entry that would now repeat the one before it is dropped instead.
 */
export function replacePlace(h: PaneHistory, ref: PaneRef): PaneHistory {
    if (h.index < 0) return recordPane(h, ref);
    if (samePlace(h.entries[h.index], ref)) return h;
    if (h.index > 0 && samePlace(h.entries[h.index - 1], ref)) {
        const entries = h.entries.filter((_, k) => k !== h.index);
        return { entries, index: h.index - 1 };
    }
    const entries = h.entries.map((e, k) => (k === h.index ? { ...ref } : e));
    return { entries, index: h.index };
}

/** Entry `i` with its view place changed (or removed with undefined): the place read back after a restore. */
export function setViewAt(h: PaneHistory, i: number, view: ViewPlace | undefined): PaneHistory {
    const entry = h.entries[i];
    if (!entry || sameView(entry.view, view)) return h;
    const { view: _old, ...pane } = entry;
    const entries = h.entries.map((e, k) => (k === i ? (view ? { ...pane, view } : pane) : e));
    return { entries, index: h.index };
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
    return holder ? { ...ref, tabId: holder.tabId } : null;
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
 *
 * `current` is where the person is. When it is not the entry at the cursor (something moved them that the
 * trail did not record, an agent's move), Back goes to the entry at the cursor first, as a browser's Back
 * goes to the page its history holds; Forward goes to the entry after it. Left out, the cursor entry is
 * taken to be where the person is.
 */
export function stepHistory(
    h: PaneHistory,
    dir: -1 | 1,
    locate: (ref: PaneRef) => PaneRef | null,
    current?: PaneRef | null
): StepResult {
    const cursor = h.entries[h.index];
    const elsewhere = current != null && cursor != null && !samePlace(current, cursor);
    const dead = new Set<PaneRef>();
    let j = dir === -1 && elsewhere ? h.index : h.index + dir;
    let target: PaneRef | null = null;
    let moved: { from: PaneRef; to: PaneRef } | null = null;
    while (j >= 0 && j < h.entries.length) {
        const entry = h.entries[j];
        const where = locate(entry);
        if (where) {
            target = samePane(where, entry) ? entry : { ...entry, ...where };
            if (target !== entry) moved = { from: entry, to: target };
            break;
        }
        dead.add(entry);
        j += dir;
    }
    if (dead.size === 0 && !moved) return { history: h, target };
    const entries = h.entries.filter((e) => !dead.has(e)).map((e) => (e === moved?.from ? moved.to : e));
    // The cursor stays on its own entry; when that one was removed, on the last one left before it.
    let index = -1;
    h.entries.forEach((e, k) => {
        if (k <= h.index && !dead.has(e)) index++;
    });
    return { history: { entries, index }, target };
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
