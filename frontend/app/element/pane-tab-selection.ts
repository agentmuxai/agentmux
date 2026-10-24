// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Pane Tabs the user has picked with Ctrl+click (Cmd+click on macOS), to be
 * dragged into one pane together. One set per window, in memory only: a restart
 * clears it. It holds block ids, which are unique across panes, so a selection can
 * span panes; it belongs to the Window Tab on screen (`prunePaneTabSelection`).
 * It is explicit: a pane's active tab is not implicitly in it.
 * docs/specs/SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §4.
 */

import { createSignal } from "solid-js";

const [selection, setSelection] = createSignal<ReadonlySet<string>>(new Set());

/** The selected block ids. Reactive. */
export const paneTabSelection = selection;

/** Whether `blockId`'s pill is selected. Reactive. */
export function isPaneTabSelected(blockId: string): boolean {
    return selection().has(blockId);
}

/** Select `blockId`'s pill, or unselect it if it is selected. */
export function togglePaneTabSelected(blockId: string): void {
    const next = new Set<string>(selection());
    if (!next.delete(blockId)) next.add(blockId);
    setSelection(next);
}

/** Empty the selection. True when there was something to clear. */
export function clearPaneTabSelection(): boolean {
    if (selection().size === 0) return false;
    setSelection(new Set<string>());
    return true;
}

/**
 * Keep only the blocks in `live`: a tab that was closed, or that lives in another
 * Window Tab's layout, is no longer selected. Does nothing when nothing changes,
 * so it is cheap to call on every layout change.
 */
export function prunePaneTabSelection(live: ReadonlySet<string>): void {
    const current = selection();
    if (current.size === 0) return;
    const next = new Set<string>([...current].filter((id) => live.has(id)));
    if (next.size !== current.size) setSelection(next);
}

/**
 * The blocks a drag that starts on `blockId`'s pill carries: the whole selection
 * when that pill is in it and the selection has more than one, otherwise just
 * the pill. Not reactive: read at drag start.
 */
export function dragIdsFor(blockId: string): readonly string[] {
    const current = selection();
    return current.has(blockId) && current.size > 1 ? [...current] : [blockId];
}

/** What a header strip hands its pills to take part in the selection. */
export interface PaneTabSelectionApi {
    isSelected: (blockId: string) => boolean;
    toggle: (blockId: string) => void;
    clear: () => void;
    /** The block ids a drag starting on this pill carries. */
    dragIds: (blockId: string) => readonly string[];
}

export const paneTabSelectionApi: PaneTabSelectionApi = {
    isSelected: isPaneTabSelected,
    toggle: togglePaneTabSelected,
    clear: () => void clearPaneTabSelection(),
    dragIds: dragIdsFor,
};

/** For tests. */
export function resetPaneTabSelection(): void {
    setSelection(new Set<string>());
}
