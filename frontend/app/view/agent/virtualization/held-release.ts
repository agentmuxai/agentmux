// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, onCleanup, untrack, type Accessor } from "solid-js";
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
/**
 * How long a carried row that hasn't been on screen keeps its hold while the
 * pane is shown. Then, if it is still off screen, it is released as it would
 * have been had the reader been watching, and its text can leave memory.
 */
export const CARRY_MAX_MS = 60_000;

/** The most ids kept of rows that finished or arrived while the pane was hidden. */
export const AWAY_ARRIVALS_MAX = 200;
/** How long after a show the away rows wait for the layout to settle before they are judged anyway. */
export const AWAY_APPLY_MAX_MS = 3000;

/**
 * Rows that finished (a tool) or arrived (a jekt) while the pane was hidden.
 * Nobody watched them, so they aren't held open then (a held row keeps its
 * output in memory, #4847). Only their ids are kept, so the cost is a few
 * hundred short strings. When the pane is shown again, the ones on screen are
 * held, as they would have been had the reader been watching
 * (`awayRowsToHold`); the rest, already above the viewport, collapse as they
 * would have scrolled off.
 */
export class AwayArrivals {
    private ids = new Set<string>();
    constructor(private readonly max: number = AWAY_ARRIVALS_MAX) {}
    /** Remember a row's id (its stable node id, never an index). The oldest go past the cap. */
    add(id: string): void {
        this.ids.delete(id);
        this.ids.add(id);
        while (this.ids.size > this.max) this.ids.delete(this.ids.values().next().value as string);
    }
    get size(): number {
        return this.ids.size;
    }
    /** Every id, oldest first, and forget them. */
    take(): string[] {
        const out = [...this.ids];
        this.ids.clear();
        return out;
    }
    clear(): void {
        this.ids.clear();
    }
}

export interface AwayHoldInput {
    ids: Iterable<string>;
    held: ReadonlySet<string>;
    pinned: ReadonlySet<string>;
    viewportTop: number;
    viewportBottom: number;
    /** The row's box in the same space as the viewport, or null when it isn't rendered (virtualized away, removed, replaced). */
    rect: (id: string) => { top: number; bottom: number } | null;
}

/**
 * Which rows that arrived while the pane was hidden to hold open now: those
 * on screen. A row that isn't rendered (gone, replaced, virtualized away) or
 * is off screen is left alone, so the held set after a long absence is at most
 * the rows in view, however many arrived.
 */
export function awayRowsToHold(input: AwayHoldInput): string[] {
    const hold: string[] = [];
    for (const id of input.ids) {
        if (input.held.has(id) || input.pinned.has(id)) continue;
        const r = input.rect(id);
        if (r && r.bottom > input.viewportTop && r.top < input.viewportBottom) hold.push(id);
    }
    return hold;
}

export interface HeldReleaseInput {
    held: Iterable<string>;
    pinned: ReadonlySet<string>;
    /** Held when the pane was last shown, not yet seen on screen since. Seen ids are removed from it. */
    carried: Set<string>;
    /** The show's catch-up layout is done, so a carried row on screen now really is in view. */
    settled: boolean;
    /** The pane has been shown for CARRY_MAX_MS: carried rows still off screen are released. */
    carryExpired: boolean;
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
            } else if (input.carryExpired && (!r || r.bottom <= input.viewportTop)) {
                input.carried.delete(id);
                release.push(id);
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
    /** Rows that finished or arrived while the pane was hidden, held if on screen once it is shown. */
    away?: AwayArrivals;
    onHold?: (id: string) => void;
}): () => void {
    const dormant = useAgentDormant();
    let carried = new Set<string>();
    let shownAt = 0;
    let wasDormant = untrack(dormant);
    // Judge the rows that arrived while hidden once the show has settled.
    let awayTimer: ReturnType<typeof setTimeout> | undefined;
    const applyAway = (): void => {
        awayTimer = undefined;
        const away = opts.away;
        const scroller = opts.scroller();
        if (!away || away.size === 0 || untrack(dormant)) return; // hidden again: keep them for the next show
        const settled = performance.now() - shownAt >= RETURN_SETTLE_MS && !opts.covered();
        if (!settled && performance.now() - shownAt < AWAY_APPLY_MAX_MS) {
            awayTimer = setTimeout(applyAway, 100);
            return;
        }
        const ids = away.take();
        if (!scroller || !opts.onHold) return;
        const box = scroller.getBoundingClientRect(); // perf:allow-layout-read — once per show
        if (box.width === 0 && box.height === 0) return;
        const ds = untrack(opts.documentState);
        const hold = awayRowsToHold({
            ids,
            held: ds.heldOpenNodes,
            pinned: ds.pinnedNodes,
            viewportTop: box.top,
            viewportBottom: box.bottom,
            rect: (id) => {
                const el = scroller.querySelector(`[data-node-id="${id}"]`) as HTMLElement | null;
                return el ? el.getBoundingClientRect() : null; // perf:allow-layout-read — once per show
            },
        });
        for (const id of hold) opts.onHold(id);
    };
    createEffect(() => {
        const d = dormant();
        if (wasDormant && !d) {
            carried = new Set(untrack(opts.documentState).heldOpenNodes);
            shownAt = performance.now();
            if (awayTimer) clearTimeout(awayTimer);
            awayTimer = setTimeout(applyAway, RETURN_SETTLE_MS + 50);
        }
        if (d && awayTimer) {
            clearTimeout(awayTimer);
            awayTimer = undefined;
        }
        wasDormant = d;
    });
    onCleanup(() => {
        if (awayTimer) clearTimeout(awayTimer);
        opts.away?.clear();
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
            carryExpired: performance.now() - shownAt >= CARRY_MAX_MS,
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
