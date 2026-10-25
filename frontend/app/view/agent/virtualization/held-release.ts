// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, on, onCleanup, untrack, type Accessor } from "solid-js";
import { useAgentDormant } from "../agent-dormancy";
import type { DocumentState } from "../types";
import { SHOW_SETTLE_SAMPLE_MS, ShowSettle } from "./show-settle";

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
 * during this visit follow the plain rule. Rows a snapshot restored as held
 * (after a restart or an upgrade) are carried the same way.
 */

/**
 * How long a carried row that hasn't been on screen keeps its hold while the
 * pane is shown. Then, if it is still off screen, it is released as it would
 * have been had the reader been watching, and its text can leave memory.
 */
export const CARRY_MAX_MS = 60_000;

/** The most ids kept of rows that finished or arrived while the pane was hidden. */
export const AWAY_ARRIVALS_MAX = 200;

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
    // The show's catch-up is done when the cover is lifted and the content
    // height has stopped changing (show-settle.ts), sampled while waiting.
    const settle = new ShowSettle();
    let settleTimer: ReturnType<typeof setTimeout> | undefined;
    const stopSettleLoop = (): void => {
        if (settleTimer) clearTimeout(settleTimer);
        settleTimer = undefined;
    };
    const settleStep = (): void => {
        settleTimer = undefined;
        if (untrack(dormant)) return;
        const scroller = opts.scroller();
        // perf:allow-layout-read — one height read per sample, only until the show has settled (3 s at most)
        const height = scroller ? scroller.scrollHeight : 0;
        if (settle.sample(performance.now(), opts.covered(), height)) applyAway();
        else settleTimer = setTimeout(settleStep, SHOW_SETTLE_SAMPLE_MS);
    };
    // Judge the rows that arrived while hidden once the show has settled.
    const applyAway = (): void => {
        const away = opts.away;
        const scroller = opts.scroller();
        if (!away || away.size === 0 || untrack(dormant)) return; // hidden again: keep them for the next show
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
    // The show (or the restore) starts the wait for the layout to settle.
    const beginSettle = (): void => {
        shownAt = performance.now();
        settle.start(shownAt);
        stopSettleLoop();
        settleTimer = setTimeout(settleStep, SHOW_SETTLE_SAMPLE_MS);
    };
    // Rows a snapshot restored as held (a restart or an upgrade brought the pane
    // back): carried like rows held when the pane was hidden, so they stay open
    // until seen in this visit and then scrolled off. A pane that restores while
    // hidden takes them with the rest of its held rows when it is shown.
    createEffect(
        on(
            () => opts.documentState().restoredHeldNodes,
            (restored) => {
                if (!restored || restored.size === 0 || untrack(dormant)) return;
                for (const id of restored) carried.add(id);
                beginSettle();
            }
        )
    );
    createEffect(() => {
        const d = dormant();
        if (wasDormant && !d) {
            carried = new Set(untrack(opts.documentState).heldOpenNodes);
            beginSettle();
        }
        if (d) {
            stopSettleLoop();
            settle.cancel();
        }
        wasDormant = d;
    });
    onCleanup(() => {
        stopSettleLoop();
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
            settled: settle.settled(performance.now()) && !opts.covered(),
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
