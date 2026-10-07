// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The follower — §4 of docs/specs/SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md.
 *
 * It handles only content that arrived BELOW everything visible: it moves the
 * transcript's scrollTop toward the bottom over a few frames, forward only.
 * Anything that moved visible rows (growth above them, a clamp, a taller
 * viewport) is compensated at once by the observer callback before the
 * follower is asked to continue (one-way-flow.ts), so the follower never
 * "corrects" anything.
 *
 * The target is the bottom as last measured with layout clean, which can only
 * be at or above the real bottom, and the browser clamps a write to the real
 * bottom anyway: no write can land past the content (V2). Frame steps only
 * write; they read no layout.
 */

/** Time constant of the approach: ~140 ms to cover 90 % of a step. */
export const FOLLOW_TAU_MS = 60;
/** A step larger than this share of the viewport is shown at once (a paste, a load). */
export const SNAP_FRACTION = 0.75;
/** Within this of the target the follower is at rest. 1 px, not less: at a fractional
 *  zoom scrollHeight is a whole number and scrollTop is not, so a smaller gap may never close. */
export const REST_PX = 1;

export interface StepInput {
    pos: number;
    target: number;
    clientHeight: number;
    dtMs: number;
    reducedMotion: boolean;
}

/** The next scrollTop: never below `pos`, never above `target`. */
export function stepToward({ pos, target, clientHeight, dtMs, reducedMotion }: StepInput): number {
    const gap = target - pos;
    if (gap < REST_PX) return pos;
    if (reducedMotion || gap > clientHeight * SNAP_FRACTION) return target;
    const k = 1 - Math.exp(-Math.max(0, dtMs) / FOLLOW_TAU_MS);
    return Math.min(target, pos + Math.max(1, gap * k));
}

export interface FollowerHost {
    /** True while the pane follows and the user is not touching it. */
    active(): boolean;
    reducedMotion(): boolean;
    /** Write scrollTop; `delta` is how far it moved from the follower's last position. */
    write(top: number, delta: number): void;
    requestFrame(cb: (t: number) => void): number;
    cancelFrame(id: number): void;
}

export class Follower {
    private pos = 0;
    private target = 0;
    private clientHeight = 0;
    private raf: number | null = null;
    private lastT: number | null = null;

    constructor(private readonly host: FollowerHost) {}

    get moving(): boolean {
        return this.raf !== null;
    }

    get position(): number {
        return this.pos;
    }

    /**
     * Where things stand after an observation: the scrollTop now in effect (the
     * observer has already written any compensation) and the bottom, both read
     * with layout clean. Takes one step at once if the gap is a snap, then keeps
     * stepping on frames until at rest.
     */
    update(pos: number, target: number, clientHeight: number): void {
        this.pos = pos;
        this.target = Math.max(pos, target);
        this.clientHeight = clientHeight;
        if (!this.host.active()) return this.stop();
        const next = stepToward({ pos, target: this.target, clientHeight, dtMs: 0, reducedMotion: this.host.reducedMotion() });
        if (next > pos) {
            // A snap (reduced motion, or more than most of a viewport): land now, before paint.
            this.write(next);
        }
        if (this.target - this.pos < REST_PX) return this.stop();
        if (this.raf === null) {
            this.lastT = null;
            this.raf = this.host.requestFrame((t) => this.tick(t));
        }
    }

    /** Stop moving at once (the user took over, the pane detached or was hidden). */
    stop(): void {
        if (this.raf !== null) this.host.cancelFrame(this.raf);
        this.raf = null;
        this.lastT = null;
    }

    private tick(t: number): void {
        this.raf = null;
        if (!this.host.active()) return;
        const dt = this.lastT === null ? 16 : t - this.lastT;
        this.lastT = t;
        const next = stepToward({
            pos: this.pos,
            target: this.target,
            clientHeight: this.clientHeight,
            dtMs: dt,
            reducedMotion: this.host.reducedMotion(),
        });
        if (next > this.pos) this.write(next);
        if (this.target - this.pos < REST_PX) return;
        this.raf = this.host.requestFrame((t2) => this.tick(t2));
    }

    private write(top: number): void {
        if (top <= this.pos) return; // forward only
        const delta = top - this.pos;
        this.pos = top;
        this.host.write(top, delta);
    }
}
