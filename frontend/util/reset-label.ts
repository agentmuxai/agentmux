// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** "Sun 7:00 PM": when an agent's usage limit lifts, in this machine's time zone. */
export function resetLabel(ms: number): string {
    return new Date(ms).toLocaleString(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" });
}

/** A reset time for a window that can be days away: the weekday alone reads as today or as past, so a reset more than a day off adds the date ("Sat Nov 28, 1:00 AM"). */
export function resetLabelWithDate(ms: number, now: number = Date.now()): string {
    if (ms - now <= 24 * 3_600_000) return resetLabel(ms);
    return new Date(ms).toLocaleString(undefined, { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}
