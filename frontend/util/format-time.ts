// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared elapsed-time display formatting — consolidates 5 independently-
 * duplicated implementations found across the composer strip, footer,
 * activity dock, persistent shell block, and swarm pane.
 *
 * Two genuinely different conventions were already live in the product, so
 * this exports both rather than forcing one on every call site:
 *   - `formatElapsedCompact` — prose form ("42s", "3m 5s"), used in the
 *     composer strip and footer.
 *   - `formatElapsedClock`   — mm:ss form ("3:05"), used in the activity
 *     dock, persistent shell block, and swarm pane.
 *
 * See docs/specs/SPEC_TOKEN_STATS_NUMBER_FORMATTING_2026_08_02.md §7.1.
 */

/** Prose form: "42s" under a minute, "3m 5s" at/above a minute. */
export function formatElapsedCompact(ms: number): string {
    const s = Math.max(0, Math.floor(ms / 1000));
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

/**
 * Clock form: "M:SS", zero-padded seconds. Floors negative durations to 0 —
 * one of the 5 original copies (`PersistentShellBlock.tsx`) was missing this
 * guard and could render e.g. "-1:05" on a clock-skew edge case; the other
 * clock-style copies already had it.
 */
export function formatElapsedClock(ms: number): string {
    const totalSec = Math.max(0, Math.floor(ms / 1000));
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    return `${m}:${String(s).padStart(2, "0")}`;
}
