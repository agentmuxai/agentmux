// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a window tab's content is fully in place: its layout has loaded,
 * and every pane in it is mounted with no content hold outstanding
 * (pane-content-holds.ts). A tab in that state, kept laid out while hidden,
 * can be shown in one frame with nothing left to load on screen
 * (docs/analysis/ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md §9).
 */

import { markTabShown } from "@/app/store/tab-reveal";
import { paneContentSettled } from "@/app/store/pane-content-holds";
import { peekLayoutModelForTab } from "@/layout/index";

export function tabContentSettled(tabId: string): boolean {
    // Never create a model here: a tab whose view hasn't asked for one yet
    // isn't mounted, so it isn't settled.
    const model = peekLayoutModelForTab(tabId);
    if (!model) return false;
    // The layout itself must have loaded, or an empty leaf list means
    // "not yet", not "no panes" (Codex on #4132).
    if (!model.getter(model.muxObjectAtom)) return false;
    return model.leafs().every((leaf) => {
        const blockId = leaf.data?.blockId;
        return !blockId || paneContentSettled(blockId);
    });
}

/** Resolves true on the first frame the tab is settled, or false after `capMs`. */
export function whenTabContentSettled(tabId: string, capMs: number): Promise<boolean> {
    const start = performance.now();
    return new Promise((resolve) => {
        const check = () => {
            if (tabContentSettled(tabId)) return resolve(true);
            if (performance.now() - start >= capMs) return resolve(false);
            requestAnimationFrame(check);
        };
        check();
    });
}

/**
 * Count each tab a window loaded with as shown once its content has settled,
 * so the first switch to it is as clean as any later one. A tab that hasn't
 * settled within `capMs` keeps its first reveal gated. `keptLaidOut` is
 * re-read each frame, in case the setting changes meanwhile. Returns a
 * cancel.
 */
export function markLoadedTabsShownWhenSettled(
    tabIds: readonly string[],
    keptLaidOut: () => boolean,
    capMs = 10_000
): () => void {
    const pending = new Set(tabIds);
    const start = performance.now();
    let frame = 0;
    const check = () => {
        if (!keptLaidOut()) return;
        for (const id of [...pending]) {
            if (!tabContentSettled(id)) continue;
            markTabShown(id);
            pending.delete(id);
        }
        if (pending.size > 0 && performance.now() - start < capMs) frame = requestAnimationFrame(check);
    };
    frame = requestAnimationFrame(check);
    return () => cancelAnimationFrame(frame);
}
