// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, untrack, type Accessor } from "solid-js";
import { useAgentDormant } from "../agent-dormancy";
import type { DocumentState } from "../types";

/**
 * Which held-open rows to release (latched collapse) after the pane moved:
 * a row held open after finishing on screen collapses once it has scrolled
 * off the top. Pure, so the rule can be tested without a DOM.
 *
 * Leaving the pane and coming back must change nothing
 * (docs/reports/REPORT_AGENT_PANE_HELD_TOOLS_COLLAPSE_ON_RETURN_RCA_2026_10_17.md):
 * rows held when the pane was shown again are `carried`. Content that arrived
 * or was drawn while the pane was hidden may have pushed them up, which nobody
 * saw, so a carried row is released only after it has been on screen again in
 * this visit (once the show has `settled`) and then scrolled off. Rows held
 * during this visit follow the plain rule.
 */

/**
 * How long after a show before a carried row on screen counts as seen: the
 * content drawn on return lands within this (one-way flow's SHOW_SETTLE_MS).
 */
export const RETURN_SETTLE_MS = 400;

export interface HeldReleaseInput {
    held: Iterable<string>;
    pinned: ReadonlySet<string>;
    /** Held when the pane was last shown, not yet seen on screen since. Seen ids are removed from it. */
    carried: Set<string>;
    /** The show's catch-up layout is done, so a carried row on screen now really is in view. */
    settled: boolean;
    viewportTop: number;
    viewportBottom: number;
    /** The row's box in the same space as the viewport, or null when it isn't rendered. */
    rect: (id: string) => { top: number; bottom: number } | null;
}

export function heldRowsToRelease(input: HeldReleaseInput): string[] {
    const release: string[] = [];
    for (const id of input.held) {
        if (input.pinned.has(id)) continue; // pin wins
        const r = input.rect(id);
        if (input.carried.has(id)) {
            // Not released until seen in this visit; once seen, the plain rule.
            if (input.settled && r && r.bottom > input.viewportTop && r.top < input.viewportBottom) {
                input.carried.delete(id);
            }
            continue;
        }
        // Not rendered: it was virtualized away, so it is off screen. Or fully above the top.
        if (!r || r.bottom <= input.viewportTop) release.push(id);
    }
    return release;
}

/**
 * The scan the transcript runs after it moves (pin passes, user scrolls while
 * following): release each held row that has scrolled off the top. DOM-based,
 * so it works for virtualized and streaming-buffer rows and is zoom-safe (both
 * rects share the zoomed space). Skipped while the pane is hidden: a kept-alive
 * tab is `visibility: hidden` at full size, so its rows move as the agent works.
 */
export function useScrolledOffRelease(opts: {
    scroller: () => HTMLElement | undefined;
    documentState: Accessor<DocumentState>;
    onRelease: (id: string) => void;
    /** The show cover is up: a carried row under it isn't seen yet. */
    covered: () => boolean;
}): () => void {
    const dormant = useAgentDormant();
    let carried = new Set<string>();
    let shownAt = 0;
    let wasDormant = untrack(dormant);
    createEffect(() => {
        const d = dormant();
        if (wasDormant && !d) {
            carried = new Set(untrack(opts.documentState).heldOpenNodes);
            shownAt = performance.now();
        }
        wasDormant = d;
    });
    return () => {
        const scroller = opts.scroller();
        if (!scroller || untrack(dormant)) return;
        const ds = opts.documentState();
        if (ds.heldOpenNodes.size === 0) return;
        const box = scroller.getBoundingClientRect(); // perf:allow-layout-read — pin pass (RO) or a user scroll batch
        // A minimized window (or a display:none pane) reports 0×0 for
        // everything: nothing can be judged, so don't mass-release (#2538).
        if (box.width === 0 && box.height === 0) return;
        const ids = heldRowsToRelease({
            held: ds.heldOpenNodes,
            pinned: ds.pinnedNodes,
            carried,
            settled: performance.now() - shownAt >= RETURN_SETTLE_MS && !opts.covered(),
            viewportTop: box.top,
            viewportBottom: box.bottom,
            rect: (id) => {
                const el = scroller.querySelector(`[data-node-id="${id}"]`) as HTMLElement | null;
                return el ? el.getBoundingClientRect() : null; // perf:allow-layout-read — pin pass (RO) or a user scroll batch
            },
        });
        for (const id of ids) opts.onRelease(id);
    };
}
