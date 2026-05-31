// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Predictive local echo — paint a just-typed printable character in the same
 * frame as the keydown, before the authoritative PTY echo completes its
 * cross-process round-trip (the gap vs VS Code's in-process echo).
 *
 * Spec: docs/specs/SPEC_TERMINAL_PREDICTIVE_LOCAL_ECHO_2026_05_31.md
 *
 * Safety contract (spec §4):
 *  - Authoritative output is never altered; predictions are a reconcilable
 *    overlay and the buffer always converges to exactly what the PTY sent.
 *  - Nothing is predicted without POSITIVE evidence echo is on: we require a
 *    byte-exact confirmation of our own input before arming (spec §7.1), so a
 *    password prompt (echo off) never flashes plaintext — the first keystroke
 *    of any burst is authoritative-only, and confirmations stop the instant
 *    echo stops.
 *  - Reconciliation is byte-exact; any divergence rolls back ALL outstanding
 *    predictions and enters cooldown.
 *  - RTT-gated: dormant when the round-trip is already faster than the budget,
 *    so enabling it is a no-op where there's nothing to hide.
 *
 * This module is the pure state machine; the xterm interaction is injected via
 * `PredictSink` so it is unit-testable without a terminal.
 */

/** xterm-facing side effects, injected for testability. */
export interface PredictSink {
    /** Paint one predicted glyph at the cursor (xterm `write`); advances cursor. */
    paint(glyph: string): void;
    /** Erase the last `count` predicted glyphs, restoring authoritative state.
     *  Phase 1 appends at the cursor, so this is `CSI <n> D` + `CSI K`. */
    erase(count: number): void;
}

export interface PredictiveEchoOptions {
    /** Master enable (read live so the setting can toggle without remount). */
    enabled: () => boolean;
    /** Predict only when rolling p50 round-trip ≥ this (ms). Default 12 (~¾ frame). */
    thresholdMs?: number;
    /** Unconfirmed predictions older than this (ms) are rolled back. Default 600. */
    predictTimeoutMs?: number;
    /** After a rollback, stay unarmed this long (ms). Default 1200. */
    cooldownMs?: number;
    /** Hard cap on outstanding predictions before flushing. Default 40. */
    maxQueue?: number;
    /** Clock injection for tests. Default `performance.now`. */
    now?: () => number;
}

interface Pending {
    /** Exact bytes we expect the PTY to echo to confirm this entry. */
    expected: string;
    /** When the input was sent (for RTT + timeout). */
    at: number;
    /** Whether we actually painted a glyph (vs. an arming-only observation). */
    painted: boolean;
}

/** Phase 1 safe set: exactly one printable ASCII char (spec §6.1). CJK / wide /
 *  control / escape / multi-char are left to the authoritative path. */
export function isPredictable(data: string): boolean {
    if (data.length !== 1) return false;
    const c = data.charCodeAt(0);
    return c >= 0x20 && c <= 0x7e;
}

export class PredictiveEcho {
    private queue: Pending[] = [];
    private armed = false;
    private cooldownUntil = 0;
    private rttSamples: number[] = [];
    private rttP50 = Infinity;

    private readonly threshold: number;
    private readonly predictTimeout: number;
    private readonly cooldown: number;
    private readonly maxQueue: number;
    private readonly now: () => number;

    constructor(
        private readonly sink: PredictSink,
        private readonly opts: PredictiveEchoOptions,
    ) {
        this.threshold = opts.thresholdMs ?? 12;
        this.predictTimeout = opts.predictTimeoutMs ?? 600;
        this.cooldown = opts.cooldownMs ?? 1200;
        this.maxQueue = opts.maxQueue ?? 40;
        this.now = opts.now ?? (() => performance.now());
    }

    /** Call AFTER sending `data` to the PTY (spec §6, `handleTermData`). */
    onInput(data: string): void {
        if (!this.opts.enabled()) {
            this.reset();
            return;
        }
        if (data.length === 0) return;
        // Anything outside the safe set (Enter, arrows, Ctrl-*, ESC, paste,
        // multi-char) flushes: roll back any painted predictions and stop — the
        // authoritative stream will redraw whatever the shell does.
        if (!isPredictable(data)) {
            this.flush();
            return;
        }
        const now = this.now();
        // Observe-only (no paint) while in cooldown, unarmed, or when the
        // round-trip is already fast enough that prediction buys nothing.
        if (now < this.cooldownUntil || !this.armed || this.rttP50 < this.threshold) {
            this.observe(data, now);
            return;
        }
        // Armed + safe + slow enough → predict.
        this.sink.paint(data);
        this.queue.push({ expected: data, at: now, painted: true });
        if (this.queue.length > this.maxQueue) this.flush();
    }

    /**
     * Reconcile an authoritative PTY chunk against outstanding predictions
     * (spec §6.3). Call BEFORE writing to xterm; returns the bytes the caller
     * must write authoritatively:
     *  - a PAINTED prediction's echo is CONSUMED (the glyph is already on screen);
     *  - an arming OBSERVATION's echo is PASSED THROUGH (nothing was painted, so
     *    the real echo must still be written);
     *  - on the first divergence, ALL outstanding painted predictions roll back
     *    and the remaining bytes (incl. the diverging bytes) pass through.
     * The visible buffer therefore always converges to exactly the PTY stream.
     */
    reconcile(chunk: string): string {
        if (this.queue.length === 0) return chunk;
        const now = this.now();
        let rest = chunk;
        let auth = "";
        while (this.queue.length > 0 && rest.length > 0) {
            const head = this.queue[0];
            if (rest.startsWith(head.expected)) {
                this.queue.shift();
                this.recordRtt(now - head.at);
                this.armed = true; // observed our own echo → safe to predict
                if (!head.painted) auth += head.expected; // observation: must still write
                rest = rest.slice(head.expected.length);
            } else {
                this.rollback();
                this.enterCooldown(now);
                auth += rest;
                rest = "";
                break;
            }
        }
        return auth + rest;
    }

    /** Time out unconfirmed predictions (spec §6.3 step 3) — the echo-off /
     *  password-prompt catch. Call on a cheap cadence (e.g. each reconcile). */
    sweep(): void {
        if (this.queue.length === 0) return;
        const now = this.now();
        if (now - this.queue[0].at > this.predictTimeout) {
            this.rollback();
            this.enterCooldown(now);
        }
    }

    /** Rolling p50 round-trip (ms); Infinity until the first confirmation. */
    get roundTripP50(): number {
        return this.rttP50;
    }

    /** Outstanding (unconfirmed) prediction count — for diagnostics/tests. */
    get pending(): number {
        return this.queue.length;
    }

    get isArmed(): boolean {
        return this.armed;
    }

    /** Live master-enable read (the `term:predictiveecho` setting). */
    isEnabled(): boolean {
        return this.opts.enabled();
    }

    /** Drop everything (resize, disconnect, disable). */
    reset(): void {
        if (this.painted() > 0) this.sink.erase(this.painted());
        this.queue = [];
        this.armed = false;
    }

    // ── internals ───────────────────────────────────────────────────────────

    private observe(data: string, now: number): void {
        // Enqueue an expectation WITHOUT painting — bootstraps arming + RTT from
        // the real echo while showing nothing speculative.
        this.queue.push({ expected: data, at: now, painted: false });
        if (this.queue.length > this.maxQueue) this.queue.shift();
    }

    private painted(): number {
        let n = 0;
        for (const p of this.queue) if (p.painted) n++;
        return n;
    }

    private rollback(): void {
        const n = this.painted();
        if (n > 0) this.sink.erase(n);
        this.queue = [];
    }

    private flush(): void {
        this.rollback();
    }

    private enterCooldown(now: number): void {
        this.armed = false;
        this.cooldownUntil = now + this.cooldown;
    }

    private recordRtt(ms: number): void {
        this.rttSamples.push(ms);
        if (this.rttSamples.length > 64) this.rttSamples.shift();
        const s = [...this.rttSamples].sort((a, b) => a - b);
        this.rttP50 = s[(s.length - 1) >> 1];
    }
}
