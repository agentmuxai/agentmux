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
 * Clock form, growing a field at a time as the duration does:
 * `M:SS` → `H:MM:SS` → `D:HH:MM:SS`.
 *
 * The hour and day fields are not cosmetic. This drives the long-running
 * process dock, whose whole purpose is processes that outlive a turn, and the
 * minutes-only form silently kept counting past 60 — an hour-old `task dev`
 * read "75:03", which is not a duration anyone can parse at a glance, and a
 * day-old one read "1508:22".
 *
 * Fields are added rather than always padded (no "0:00:05" for a five-second
 * tool call) because the overwhelmingly common case is seconds-to-minutes, and
 * the dock shows these inline next to a title where width matters. Same
 * convention as ffmpeg/YouTube timestamps.
 *
 * Floors negative durations to 0 — one of the 5 original copies
 * (`PersistentShellBlock.tsx`) was missing this guard and could render e.g.
 * "-1:05" on a clock-skew edge case; the other clock-style copies already
 * had it.
 */
export function formatElapsedClock(ms: number): string {
    const totalSec = Math.max(0, Math.floor(ms / 1000));
    const s = totalSec % 60;
    const m = Math.floor(totalSec / 60) % 60;
    const h = Math.floor(totalSec / 3_600) % 24;
    const d = Math.floor(totalSec / 86_400);

    const ss = String(s).padStart(2, "0");
    if (d > 0) return `${d}:${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${ss}`;
    if (h > 0) return `${h}:${String(m).padStart(2, "0")}:${ss}`;
    return `${m}:${ss}`;
}

/**
 * Relative "time ago" form — consolidates 6+ independently-duplicated
 * implementations (`MyAgentsList.tsx`, `AgentLaunchModal.tsx`,
 * `AgentDisconnectedBanner.tsx`, `swarm-view.tsx`, `usenotification.tsx`,
 * `warden.tsx`, plus devtools-only copies). Migrated from
 * `MyAgentsList.tsx`'s `formatRelative` (the one with existing test
 * coverage) — same behavior, `now` reordered to a trailing, defaulted
 * parameter so production call sites don't need to pass it explicitly
 * while tests still can.
 *
 * See docs/specs/SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md §2.2.
 */
export function formatTimeAgo(ms: number, now: number = Date.now()): string {
    if (!ms) return "";
    const delta = Math.max(0, now - ms);
    if (delta < 60_000) return "just now";
    if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
    if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
    return `${Math.floor(delta / 86_400_000)}d ago`;
}

/**
 * Absolute local time, 12-hour with an AM/PM suffix — "3:46:12 PM".
 *
 * Was "HH:MM:SS" (24-hour) until 2026-08-30; see the inline note in the
 * body for what did and didn't change. Adapted from the unshipped
 * `docs/specs/node-timestamp-hover.md`'s draft — dropped the tenths-of-a-
 * second digit that draft used (not meaningful at the granularity this is
 * actually used at: hovering a tool call or thinking clump, not diagnosing
 * out-of-order sub-second delivery).
 */
export function formatExactTime(ms: number): string {
    const d = new Date(ms);
    // `getHours()` is already LOCAL time — this was never UTC. What changed
    // (2026-08-30, operator request) is the presentation: 12-hour with an
    // AM/PM suffix instead of a 24-hour clock. Hour is NOT zero-padded, per
    // the usual 12-hour convention ("3:46:12 PM", not "03:46:12 PM");
    // minutes and seconds still are.
    const h24 = d.getHours();
    const h = h24 % 12 === 0 ? 12 : h24 % 12;
    const m = String(d.getMinutes()).padStart(2, "0");
    const s = String(d.getSeconds()).padStart(2, "0");
    return `${h}:${m}:${s} ${h24 < 12 ? "AM" : "PM"}`;
}
