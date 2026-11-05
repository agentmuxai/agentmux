// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which drop a pane drag means, steadied. The raw answer of `placementDirection` flips on every sample that
 * crosses a diagonal or a pane border, so a pointer resting on one makes the ghost (and, once the other panes
 * move to make room, the panes) shuffle. The tracker keeps the candidate it has armed until the pointer has
 * clearly left it: it must be `marginPx` away from the last place the old candidate still held, and then
 * either be moving slowly (aiming) or have held the new candidate for `dwellMs`. Pure, with the clock passed
 * in. docs/reports/REPORT_LIVE_REFLOW_PANE_DRAG_ALGORITHMS_2026_11_05.md §4.
 */

import type { DropDirection } from "./types";

export interface Candidate {
    targetId: string;
    direction: DropDirection | undefined;
}

export interface TrackerConfig {
    /** How far from the last place the armed candidate held a new one may win, px. */
    marginPx: number;
    /** How long a new candidate must hold before it arms while the pointer is still moving fast, ms. 0: at once. */
    dwellMs: number;
    /** Below this pointer speed the user is aiming, and a new candidate arms at once, px per ms. */
    slowPxPerMs: number;
    /** A gap longer than this between samples starts a new drag, ms. */
    staleMs: number;
}

export const DEFAULT_TRACKER: TrackerConfig = { marginPx: 8, dwellMs: 120, slowPxPerMs: 0.35, staleMs: 1500 };

export interface Step {
    /** The candidate to show now. */
    armed: Candidate;
    /** When the sample's candidate will arm if no further sample comes, or undefined if nothing is waiting. */
    flushAt?: number;
}

const same = (a: Candidate, b: Candidate) => a.targetId === b.targetId && a.direction === b.direction;

export class DropIntentTracker {
    private armed: Candidate | null = null;
    private draggedId: string | null = null;
    /** The last pointer place where the armed candidate was also the raw answer. */
    private heldAt = { x: 0, y: 0 };
    private last: { x: number; y: number; t: number } | null = null;
    private speed = 0;
    private waiting: { candidate: Candidate; since: number } | null = null;

    constructor(private readonly config: TrackerConfig = DEFAULT_TRACKER) {}

    /** Take one sample of `raw` at pointer (x, y), time t (ms), for the drag of `draggedId`. */
    step(draggedId: string, x: number, y: number, t: number, raw: Candidate): Step {
        if (this.draggedId !== draggedId || (this.last && t - this.last.t > this.config.staleMs)) this.reset();
        this.draggedId = draggedId;
        this.noteSpeed(x, y, t);
        if (!this.armed) return this.arm(raw, x, y);
        if (same(this.armed, raw)) {
            this.heldAt = { x, y };
            this.waiting = null;
            return { armed: this.armed };
        }
        if (Math.hypot(x - this.heldAt.x, y - this.heldAt.y) < this.config.marginPx) {
            this.waiting = null;
            return { armed: this.armed };
        }
        if (!this.waiting || !same(this.waiting.candidate, raw)) this.waiting = { candidate: raw, since: t };
        if (this.speed < this.config.slowPxPerMs || t - this.waiting.since >= this.config.dwellMs) {
            return this.arm(raw, x, y);
        }
        return { armed: this.armed, flushAt: this.waiting.since + this.config.dwellMs };
    }

    /** The waiting candidate once its dwell is over with no new sample; the armed one otherwise. */
    flush(t: number): Candidate | null {
        if (this.waiting && t - this.waiting.since >= this.config.dwellMs) {
            this.armed = this.waiting.candidate;
            this.waiting = null;
        }
        return this.armed;
    }

    /** A new drag, or the end of one. */
    reset(): void {
        this.armed = null;
        this.draggedId = null;
        this.last = null;
        this.speed = 0;
        this.waiting = null;
    }

    private arm(candidate: Candidate, x: number, y: number): Step {
        this.armed = candidate;
        this.heldAt = { x, y };
        this.waiting = null;
        return { armed: candidate };
    }

    private noteSpeed(x: number, y: number, t: number): void {
        if (this.last && t > this.last.t) {
            const instant = Math.hypot(x - this.last.x, y - this.last.y) / (t - this.last.t);
            this.speed = this.speed * 0.5 + instant * 0.5;
        }
        this.last = { x, y, t };
    }
}
