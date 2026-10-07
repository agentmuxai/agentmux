// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One-way flow for a following transcript — R2–R4 of
 * docs/specs/SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md.
 *
 * - R2, compensate: in one ResizeObserver callback, with layout clean, find how
 *   far each row that was visible at the last observation moved on screen, and
 *   scroll forward at once by the largest downward move. Every visible row then
 *   sits at or above where it was, whatever moved it.
 * - R3, a bottom spacer: a compensating scroll needs room below it when the
 *   content got shorter or the viewport taller. A trailing element provides
 *   exactly that much; new content fills it, and it only shrinks while it is
 *   below the viewport.
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
 * (spec §1), so for this long they land at the true bottom instead of being
 * held in place with spacer room, which left a gap between the user's message
 * and the composer for the agent to fill slowly.
 */
export const COMMIT_SETTLE_MS = 600;

/** Below this, a move is sub-pixel noise, not something to compensate. */
export const MOVE_EPSILON_PX = 0.5;

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

export interface SpacerInput {
    /** The spacer's current height, px. */
    spacer: number;
    /** The scrollTop that must stay reachable. */
    wanted: number;
    clientHeight: number;
    /** scrollHeight now, including the current spacer. */
    scrollHeight: number;
}

/**
 * The spacer height that keeps `wanted` reachable with nothing to spare:
 * grows when the content got shorter or the viewport taller, is used up first
 * when content grows, never negative. Rounded up so a fractional `wanted`
 * against an integer scrollHeight never clamps by a fraction of a pixel.
 */
export function spacerFor({ spacer, wanted, clientHeight, scrollHeight }: SpacerInput): number {
    return Math.max(0, spacer + Math.ceil(wanted + clientHeight - scrollHeight - 0.001));
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
    private spacerEl: HTMLElement | null = null;
    private spacerPx = 0;
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
    /** Until when layout changes settle at the bottom without spacer room (see COMMIT_SETTLE_MS). */
    private settleUntil = Number.NEGATIVE_INFINITY;
    /** scrollHeight / clientHeight at the last observation, for the geometry of frame-step writes. */
    private lastGeo = { scrollHeight: 0, clientHeight: 0 };
    private readonly follower: Follower;
    private disposed = false;
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

    /** The scroller, the spacer, and the containers whose size is "how tall the content is". */
    attach(scroller: HTMLElement, spacer: HTMLElement, containers: (Element | undefined)[]): void {
        this.scroller = scroller;
        this.spacerEl = spacer;
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
     * keystroke) it is a commit point (spec §1): any spacer room is given up,
     * and layout changes for COMMIT_SETTLE_MS after it settle at the bottom
     * too. Typing doesn't open the window, or the one-way rule would be off
     * for as long as the user types.
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
        this.baseline = this.readVisible(el, ch);
    }

    /**
     * A commit point without a jump (a queued message accepted mid-turn): give
     * up spacer room and let layout changes for COMMIT_SETTLE_MS settle at the
     * bottom. Callers only use it while the pane follows.
     */
    commit(): void {
        this.settleUntil = performance.now() + COMMIT_SETTLE_MS;
        if (this.spacerPx > 0) {
            this.host.note?.(`spacer:release-${this.spacerPx}`);
            this.setSpacer(0);
        }
    }

    /** `/clear` or a fresh session in the pane: no room is owed to anything. */
    reset(): void {
        this.setSpacer(0);
        this.baseline = null;
        this.follower.stop();
    }

    /**
     * A scroll batch the user caused (the scroll handler, inside the user-input
     * window, live geometry). Re-baselines the visible rows, lets go of spacer
     * room that is now below the viewport, and returns true when the user moved
     * the transcript up while following: that releases follow at once, however
     * small the move.
     */
    userScrolled(geo: { scrollTop: number; scrollHeight: number; clientHeight: number }): boolean {
        const el = this.scroller;
        if (!el) return false;
        this.follower.stop();
        const movedUp = geo.scrollTop < this.follower.position - 1;
        if (this.spacerPx > 0) {
            this.setSpacer(spacerFor({ spacer: this.spacerPx, wanted: geo.scrollTop, clientHeight: geo.clientHeight, scrollHeight: geo.scrollHeight }));
        }
        this.baseline = this.readVisible(el, geo.clientHeight);
        return movedUp && this.host.following();
    }

    private active(): boolean {
        return !this.disposed && this.host.following() && !this.host.userActive();
    }

    private onResize(): void {
        const el = this.scroller;
        if (!el || this.disposed || !el.isConnected) return;
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
        let sh = el.scrollHeight; // perf:allow-layout-read — ResizeObserver callback (layout clean)
        if (cw !== this.lastWidth) {
            // Width or zoom changed (user-initiated): everything reflowed, nothing
            // to compare against, and no room is owed. Start over at the bottom.
            const first = this.lastWidth < 0;
            this.lastWidth = cw;
            this.baseline = null;
            if (!first && this.spacerPx > 0) {
                this.setSpacer(0);
                sh = el.scrollHeight; // perf:allow-layout-read — once, after a width change
            }
        }
        if (this.active() && performance.now() < this.settleUntil) {
            // Just after the user committed: their own layout changes move the
            // content, and we stay at the true bottom with no room held.
            if (this.spacerPx > 0) {
                sh -= this.spacerPx;
                this.setSpacer(0);
            }
            const bottom = Math.max(0, sh - ch);
            if (Math.abs(bottom - live) > MOVE_EPSILON_PX) {
                el.scrollTop = bottom;
                live = el.scrollTop; // perf:allow-layout-read — after the spacer write: one layout, inside the RO callback
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
                    wanted = live + d;
                    this.host.note?.(`comp:${Math.round(d)}`);
                }
            }
            const spacer = spacerFor({ spacer: this.spacerPx, wanted, clientHeight: ch, scrollHeight: sh });
            if (spacer !== this.spacerPx) {
                if (spacer > this.spacerPx) this.host.note?.(`spacer:+${spacer - this.spacerPx}`);
                sh += spacer - this.spacerPx;
                this.setSpacer(spacer);
            }
            if (wanted > live + MOVE_EPSILON_PX) {
                el.scrollTop = wanted;
                live = el.scrollTop; // perf:allow-layout-read — after the spacer write: one layout, inside the RO callback
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
            // Detached: keep only the room the reader's position needs.
            if (this.spacerPx > 0) {
                const spacer = spacerFor({ spacer: this.spacerPx, wanted: live, clientHeight: ch, scrollHeight: sh });
                if (spacer !== this.spacerPx) {
                    sh += spacer - this.spacerPx;
                    this.setSpacer(spacer);
                }
            }
        }
        this.lastGeo = { scrollHeight: sh, clientHeight: ch };
        this.baseline = this.readVisible(el, ch);
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

    private setSpacer(px: number): void {
        this.spacerPx = px;
        if (this.spacerEl) this.spacerEl.style.height = px > 0 ? `${px}px` : "";
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
    private readVisible(el: HTMLElement, _clientHeight: number): Map<string, number> {
        const out = new Map<string, number>();
        for (const [id, r] of this.readRows(el)) out.set(id, r.top);
        return out;
    }
}
