// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Detects a wedged terminal parser.
 *
 * Why this exists: an uncaught exception inside xterm's escape-sequence parser
 * aborts `parse()` partway through a chunk and leaves the terminal mid-state —
 * it stops rendering and stops accepting input, with nothing surfaced to the
 * user. That is exactly what a minifier bug in xterm 6.0.0's DECRQM handler did
 * (`docs/retro/retro-xterm-requestmode-minify-freeze-2026-09-17.md`): a dead
 * pane, no error in the UI, and a diagnosis that took far longer than it should
 * have.
 *
 * That specific bug is fixed. This guards the *class*: any future throw, or a
 * handler that hangs, wedges a pane the same silent way.
 *
 * Detection is deliberately cause-agnostic. Sniffing `error.stack` for xterm
 * frames is unreliable against minified vendor code — the very condition we are
 * trying to survive. Instead we watch liveness: `Terminal.write(data, cb)`
 * invokes `cb` once that chunk has been parsed, so writes that are handed over
 * and never acknowledged mean the parser stopped consuming. That signal holds
 * regardless of whether the cause was an exception, an infinite loop, or a
 * handler that never returns.
 */

/** Generous: slow first paint and large pastes are normal, a wedge is not. */
export const WEDGE_TIMEOUT_MS = 5000;

/** Don't thrash a pane that keeps failing — recovery is disruptive. */
export const WEDGE_RECOVERY_COOLDOWN_MS = 30000;

export interface WedgeState {
    /** Writes handed to xterm whose callback has not fired. */
    pending: number;
    /** When the most recent write callback fired (or writes began). */
    lastSettleTs: number;
    /** When we last attempted recovery, or 0. */
    lastRecoveryTs: number;
}

export function newWedgeState(now: number): WedgeState {
    return { pending: 0, lastSettleTs: now, lastRecoveryTs: 0 };
}

export function onWrite(s: WedgeState, now: number): void {
    // First write of a burst re-arms the clock: `lastSettleTs` means "last time
    // we had evidence the parser was alive", and an idle terminal is alive.
    if (s.pending === 0) s.lastSettleTs = now;
    s.pending++;
}

export function onSettle(s: WedgeState, now: number): void {
    if (s.pending > 0) s.pending--;
    s.lastSettleTs = now;
}

/**
 * Should we try to recover right now?
 *
 * Requires BOTH unacknowledged writes and silence past the timeout: either
 * alone is normal (an idle pane has no pending writes; a busy one settles).
 */
export function shouldRecover(
    s: WedgeState,
    now: number,
    timeoutMs: number = WEDGE_TIMEOUT_MS,
    cooldownMs: number = WEDGE_RECOVERY_COOLDOWN_MS
): boolean {
    if (s.pending === 0) return false;
    if (now - s.lastSettleTs < timeoutMs) return false;
    if (s.lastRecoveryTs !== 0 && now - s.lastRecoveryTs < cooldownMs) return false;
    return true;
}

export function markRecovered(s: WedgeState, now: number): void {
    s.lastRecoveryTs = now;
    // Drop the backlog: those callbacks will never arrive, and leaving them
    // counted would re-trigger recovery the moment the cooldown lapses.
    s.pending = 0;
    s.lastSettleTs = now;
}
