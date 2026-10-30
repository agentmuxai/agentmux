// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One-way flow for a following transcript — R2 and R4 of
 * docs/specs/SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md, and the tight rule of
 * docs/specs/SPEC_AGENT_PANE_BOTTOM_TIGHT_INVARIANT_2026_10_29.md.
 *
 * - R2, compensate growth: in one ResizeObserver callback, with layout clean,
 *   find how far each row that was visible at the last observation moved on
 *   screen, and scroll forward at once by the largest downward move, never past
 *   the true bottom. Growth makes its own room, so every visible row then sits
 *   at or above where it was.
 * - T, tight: while following there is never room below the latest content.
 *   When the content below the reader got shorter, or the scroller taller, the
 *   rows above slide down by exactly that much (the browser's own clamp) and the
 *   bottom stays on the bottom edge. The old R3 bottom spacer, which kept the
 *   rows still by leaving that much blank, is gone. A change of the scroller's
 *   height or width (a pane resize, zoom) lands at the bottom at once.
 * - R4, one observer and one writer: this module owns the only observer and the
 *   only scrollTop writes on the one-way path; user input suspends both.
 *
 * Content arriving below everything visible is left to the follower
 * (follower.ts), which eases toward the new bottom.
 */

import { Follower } from "./follower";

/** How often to look again while the user is touching a following pane: just past the 250 ms input window. */
export const RECHECK_MS = 300;

/**
 * After the user commits (a send, or a queued message starting its turn), the
 * layout keeps changing for a moment: the composer shrinks back to one line,
 * the queued-message panel comes and goes. Those are the user's own changes
 * (spec §1), so for this long they land at the true bottom at once instead of
 * being compensated row by row.
 */
const COMMIT_SETTLE_MS = 600;

/**
 * After the pane is shown again (its tab was hidden), the content it got while
 * hidden commits in stages: its text, about 90 ms later the text's settled
 * render, a tool log's 150 ms height transition. For this long each stage lands
 * at the bottom before paint (`settleOnShow`).
 */
export const SHOW_SETTLE_MS = 400;

/** Below this, a move is sub-pixel noise, not something to compensate. */
const MOVE_EPSILON_PX = 0.5;

/** The largest downward move (px, ≥ 0) of any row present in both maps. */
export function largestDownwardMove<K>(before: ReadonlyMap<K, number>, after: ReadonlyMap<K, number>): number {
    let worst = 0;
    for (const [k, top] of before) {
        const now = after.get(k);
        if (now === undefined) continue;
        const dy = now - top;
        if (dy > worst) worst = dy;
    }
    return worst > MOVE_EPSILON_PX ? worst : 0;
}

export interface OneWayHost {
    /** The pane follows (stick-to-bottom). */
    following(): boolean;
    /** The user is touching the pane (scroll input window, a held pointer). */
    userActive(): boolean;
    reducedMotion(): boolean;
    /** Our own scrollTop write, with the geometry it produced (the scroll handler trusts it). */
    wrote(geo: { scrollTop: number; scrollHeight: number; clientHeight: number }): void;
    /**
     * Before every observation is measured: the host's own layout changes that
     * belong to this frame (collapsing a held-open tool that scrolled off the
     * top). Run first so whatever they move is compensated in the same frame.
     */
    prepare?(): void;
    /** After every observation: overflow state, the recorder's sample. Must not change layout. */
    observed?(geo: { scrollTop: number; scrollHeight: number; clientHeight: number }): void;
    /** Recorder note (one-way-recorder.ts). */
    note?(label: string): void;
}

export class OneWayFlow {
    private scroller: HTMLElement | null = null;
    private readonly ro: ResizeObserver | null;
    private readonly rows = new Set<Element>();
    /**
     * Screen top (CSS px from the viewport's top edge) of every mounted row at
     * the last observation, by node id, not by element: a row that moves from
     * the live tail into the virtualized head is a new element at a possibly
     * different spot, and that move must be compared like any other. Every row,
     * not only the visible ones: the follower can scroll a row into view
     * between observations (no resize, so no observation), and a later growth
     * above it must still be caught.
     */
    private baseline: Map<string, number> | null = null;
    private lastWidth = -1;
    /** The scroller's clientHeight at the last observation: a change is a resize, which lands at the bottom at once. */
    private lastHeight = -1;
    /** Until when layout changes land at the bottom at once (see COMMIT_SETTLE_MS, SHOW_SETTLE_MS). */
    private settleUntil = Number.NEGATIVE_INFINITY;
    /** scrollHeight / clientHeight at the last observation, for the geometry of frame-step writes. */
    private lastGeo = { scrollHeight: 0, clientHeight: 0 };
    private readonly follower: Follower;
    private disposed = false;
    /**
     * The pane is not on screen (its tab or window is hidden), told by `useSettleOnShow`. A kept-alive hidden tab
     * stays laid out at full size, so its observers still fire and its follower would keep writing scrollTop every
     * frame for a pane nobody sees: neither runs while this is true. `settleOnShow` brings the pane to its bottom.
     */
    private hidden = false;
    /** A re-check scheduled while the user touched a following pane (see onResize). */
    private recheckTimer: ReturnType<typeof setTimeout> | undefined;

    constructor(private readonly host: OneWayHost) {
        this.ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(() => this.onResize()) : null;
        this.follower = new Follower({
            active: () => this.active(),
            reducedMotion: () => host.reducedMotion(),
            write: (top, delta) => this.writeScroll(top, delta),
            requestFrame: (cb) => requestAnimationFrame(cb),
            cancelFrame: (id) => cancelAnimationFrame(id),
        });
    }

    /** The scroller, and the containers whose size is "how tall the content is". */
    attach(scroller: HTMLElement, containers: (Element | undefined)[]): void {
        this.scroller = scroller;
        this.ro?.observe(scroller);
        for (const c of containers) if (c) this.ro?.observe(c, { box: "border-box" });
    }

    observeRow(el: Element): void {
        this.rows.add(el);
        // Border box, not the default content box: a row whose padding or
        // border changes (a tool's status styling) moves every row below it
        // without its content box changing size.
        this.ro?.observe(el, { box: "border-box" });
    }

    unobserveRow(el: Element): void {
        this.rows.delete(el);
        this.ro?.unobserve(el);
    }

    dispose(): void {
        this.disposed = true;
        clearTimeout(this.recheckTimer);
        this.follower.stop();
        this.ro?.disconnect();
        this.rows.clear();
    }

    /**
     * Typing, send, a queued turn, the jump button: straight to the bottom.
     * With `commit` (a send, a queued message starting its turn; not each
     * keystroke) it is a commit point (spec §1): layout changes for
     * COMMIT_SETTLE_MS after it settle at the bottom too. Typing doesn't open
     * the window, or the one-way rule would be off for as long as the user
     * types.
     */
    jumpToBottom(commit = false): void {
        const el = this.scroller;
        if (!el) return;
        this.follower.stop();
        if (commit) this.commit();
        const ch = el.clientHeight; // perf:allow-layout-read — user-initiated jump
        const sh = el.scrollHeight; // perf:allow-layout-read — user-initiated jump
        const target = Math.max(0, sh - ch);
        if (target > el.scrollTop + MOVE_EPSILON_PX) { // perf:allow-layout-read — user-initiated jump
            el.scrollTop = target;
            this.host.wrote({ scrollTop: el.scrollTop, scrollHeight: sh, clientHeight: ch }); // perf:allow-layout-read — after a scrollTop write, which does not invalidate layout
        }
        this.baseline = this.readAllTops(el);
    }

    /**
     * A commit point without a jump (a queued message accepted mid-turn): let
     * layout changes for COMMIT_SETTLE_MS settle at the bottom. Callers only use
     * it while the pane follows.
     */
    commit(): void {
        this.settleUntil = performance.now() + COMMIT_SETTLE_MS;
    }

    /**
     * The pane was shown again after its tab was hidden: land at the bottom now,
     * and let layout changes for SHOW_SETTLE_MS land there too instead of being
     * eased to, so the pane appears at its latest content with no catch-up
     * motion. Output after that eases in as usual. Callers only use it while
     * the pane follows.
     * SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §3.2.
     */
    settleOnShow(): void {
        this.follower.stop();
        const el = this.scroller;
        const acting = !!el && this.active();
        this.host.note?.(acting ? "show-settle" : "show-settle:not-following");
        if (!el || !acting) return;
        // The window counts from the first painted frame after the show too, not only from
        // this call: drawing what the pane got while hidden can take a few hundred ms before
        // anything is painted (measured in the lab), which used up a window started here.
        const arm = (): void => {
            this.settleUntil = Math.max(this.settleUntil, performance.now() + SHOW_SETTLE_MS);
        };
        arm();
        if (typeof requestAnimationFrame !== "undefined") requestAnimationFrame(arm);
        const ch = el.clientHeight; // perf:allow-layout-read — once, when the pane is shown
        const sh = el.scrollHeight; // perf:allow-layout-read — once, when the pane is shown
        const target = Math.max(0, sh - ch);
        if (target > el.scrollTop + MOVE_EPSILON_PX) { // perf:allow-layout-read — once, when the pane is shown
            el.scrollTop = target;
            this.host.wrote({ scrollTop: el.scrollTop, scrollHeight: sh, clientHeight: ch }); // perf:allow-layout-read — after a scrollTop write, which does not invalidate layout
        }
        this.baseline = this.readAllTops(el);
    }

    /** `/clear` or a fresh session in the pane: nothing to compare against. */
    reset(): void {
        this.baseline = null;
        this.follower.stop();
    }

    /**
     * A scroll batch the user caused (the scroll handler, inside the user-input
     * window, live geometry). Re-baselines the visible rows and returns true
     * when the user moved the transcript up while following: that releases
     * follow at once, however small the move.
     */
    userScrolled(geo: { scrollTop: number; scrollHeight: number; clientHeight: number }): boolean {
        const el = this.scroller;
        if (!el) return false;
        this.follower.stop();
        const movedUp = geo.scrollTop < this.follower.position - 1;
        this.baseline = this.readAllTops(el);
        return movedUp && this.host.following();
    }

    setHidden(hidden: boolean): void {
        this.hidden = hidden;
        if (hidden) {
            this.follower.stop();
            this.baseline = null;
        }
    }

    private active(): boolean {
        return !this.disposed && !this.hidden && this.host.following() && !this.host.userActive();
    }

    private onResize(): void {
        const el = this.scroller;
        if (!el || this.disposed || !el.isConnected) return;
        if (this.hidden) {
            // Nothing to keep in place and no one to see a move: only the overflow state stays current.
            this.host.observed?.({
                scrollTop: el.scrollTop, // perf:allow-layout-read — ResizeObserver callback (layout clean)
                scrollHeight: el.scrollHeight, // perf:allow-layout-read — ResizeObserver callback (layout clean)
                clientHeight: el.clientHeight, // perf:allow-layout-read — ResizeObserver callback (layout clean)
            });
            return;
        }
        this.host.prepare?.();
        // ResizeObserver callback: layout is clean, these reads are free.
        const ch = el.clientHeight; // perf:allow-layout-read — ResizeObserver callback (layout clean)
        const cw = el.clientWidth; // perf:allow-layout-read — ResizeObserver callback (layout clean)
        if (ch <= 0) {
            // Hidden (inactive tab, minimized): nothing to keep in place.
            this.baseline = null;
            this.follower.stop();
            return;
        }
        let live = el.scrollTop; // perf:allow-layout-read — ResizeObserver callback (layout clean)
        const sh = el.scrollHeight; // perf:allow-layout-read — ResizeObserver callback (layout clean)
        // A pane resize or a zoom is the user's own change (spec §1): it lands at the true
        // bottom at once. A first observation has nothing to compare against.
        let resized = false;
        if (cw !== this.lastWidth) {
            // Width or zoom changed: everything reflowed, nothing to compare against.
            resized = this.lastWidth >= 0;
            this.lastWidth = cw;
            this.baseline = null;
        }
        if (ch !== this.lastHeight) {
            resized = resized || this.lastHeight >= 0;
            this.lastHeight = ch;
        }
        if (this.active() && (resized || performance.now() < this.settleUntil)) {
            // A resize, just after the user committed, or the pane was shown again
            // (settleOnShow): layout changes land at the true bottom.
            const bottom = Math.max(0, sh - ch);
            if (Math.abs(bottom - live) > MOVE_EPSILON_PX) {
                el.scrollTop = bottom;
                live = el.scrollTop; // perf:allow-layout-read — after the write: one layout, inside the RO callback
                this.host.wrote({ scrollTop: live, scrollHeight: sh, clientHeight: ch });
            }
            this.lastGeo = { scrollHeight: sh, clientHeight: ch };
            this.follower.update(live, sh - ch, ch);
        } else if (this.active()) {
            let wanted = live;
            if (this.baseline) {
                // Only rows on screen now count. A row that moved down and is
                // on screen now was on screen before too, so this is exactly the
                // set that can show a downward move.
                const d = largestDownwardMove(this.baseline, this.readOnScreen(el, ch));
                if (d > 0) {
                    // Growth made room for the scroll back, so it can reach `d`. A
                    // shrink (or a taller scroller) did not: the rows stay where the
                    // browser's clamp put them, tight against the bottom (spec T).
                    const room = Math.max(0, sh - ch - live);
                    if (d > room + MOVE_EPSILON_PX) this.host.note?.(`tight:${Math.round(d - room)}`);
                    wanted = live + Math.min(d, room);
                    if (wanted > live) this.host.note?.(`comp:${Math.round(wanted - live)}`);
                }
            }
            if (wanted > live + MOVE_EPSILON_PX) {
                el.scrollTop = wanted;
                live = el.scrollTop; // perf:allow-layout-read — after the write: one layout, inside the RO callback
                this.host.wrote({ scrollTop: live, scrollHeight: sh, clientHeight: ch });
            }
            // The geometry any write from here reports.
            this.lastGeo = { scrollHeight: sh, clientHeight: ch };
            this.follower.update(live, sh - ch, ch);
            live = this.follower.position;
        } else {
            this.follower.stop();
            // Following, but the user is touching the pane: nothing moves now.
            // Look again once they let go, or content that arrived meanwhile
            // would sit below the bottom until some unrelated resize.
            if (this.host.following()) this.recheckSoon();
        }
        this.lastGeo = { scrollHeight: sh, clientHeight: ch };
        this.baseline = this.readAllTops(el);
        this.host.observed?.({ scrollTop: live, scrollHeight: sh, clientHeight: ch });
    }

    /** Run the observation again (the user let go of the pane). Reads layout once, outside a frame. */
    recheck(): void {
        clearTimeout(this.recheckTimer);
        this.recheckTimer = undefined;
        if (!this.disposed && this.host.following()) this.onResize();
    }

    /** Look again once the user has let go (RECHECK_MS, repeated while they still hold on). */
    recheckSoon(): void {
        if (this.recheckTimer !== undefined || this.disposed) return;
        this.recheckTimer = setTimeout(() => {
            this.recheckTimer = undefined;
            if (this.host.userActive()) this.recheckSoon();
            else this.recheck();
        }, RECHECK_MS);
    }

    private writeScroll(top: number, delta: number): void {
        const el = this.scroller;
        if (!el) return;
        el.scrollTop = top;
        // Our own move: shift the record, so it is never mistaken for content moving.
        if (this.baseline) for (const [k, v] of this.baseline) this.baseline.set(k, v - delta);
        this.host.wrote({ scrollTop: top, ...this.lastGeo });
    }

    /** Zoom of the scroller's coordinate space: rects are scaled by an ancestor CSS `zoom`, clientWidth is not. */
    private zoom(el: HTMLElement): { top: number; zoom: number } {
        const r = el.getBoundingClientRect();
        const zoom = el.offsetWidth > 0 ? r.width / el.offsetWidth : 1;
        return { top: r.top + el.clientTop * zoom, zoom: zoom || 1 };
    }

    /** A row's identity across remounts: its node id, else the element itself. */
    private static key(row: Element): string | null {
        return (row as HTMLElement).dataset?.nodeId ?? null;
    }

    /** Screen top and bottom of every mounted row, by node id. */
    private readRows(el: HTMLElement): Map<string, { top: number; bottom: number }> {
        const { top, zoom } = this.zoom(el);
        const out = new Map<string, { top: number; bottom: number }>();
        for (const row of this.rows) {
            const id = OneWayFlow.key(row);
            if (id === null || out.has(id) || !row.isConnected) continue;
            const r = row.getBoundingClientRect();
            if (r.height > 0) out.set(id, { top: (r.top - top) / zoom, bottom: (r.bottom - top) / zoom });
        }
        return out;
    }

    /** Tops of the rows on screen now. */
    private readOnScreen(el: HTMLElement, clientHeight: number): Map<string, number> {
        const out = new Map<string, number>();
        for (const [id, r] of this.readRows(el)) if (r.bottom > 0 && r.top < clientHeight) out.set(id, r.top);
        return out;
    }

    /** The record for the next comparison: every mounted row's top. */
    private readAllTops(el: HTMLElement): Map<string, number> {
        const out = new Map<string, number>();
        for (const [id, r] of this.readRows(el)) out.set(id, r.top);
        return out;
    }
}
