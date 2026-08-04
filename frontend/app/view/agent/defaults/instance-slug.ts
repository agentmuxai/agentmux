// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared slug + timestamp helpers for building per-instance working
 * directories. Kept as a standalone module so the AgentLaunchModal
 * (for preview) and the AgentViewModel (for actual launch) agree on
 * the format without importing from each other's files.
 *
 * Format: `<slug(name)>-MMDDh`, local time. Same-hour collisions are
 * resolved server-side by `agentmux-srv/src/server/app_api.rs::
 * allocate_agent_workdir`, called from `WriteAgentConfigCommand`
 * when the frontend sets `auto_allocate: true`. The atomic mkdir +
 * `<base>-N` retry there is the source of truth for collision
 * handling — this module only owns the slug format.
 */

/**
 * Slugify a user-entered name: lowercase, spaces → "-", strip
 * anything outside [a-z0-9-_]. Matches the legacy logic in
 * `AgentPicker.handleRename` so existing definitions keep the
 * same disk paths.
 */
export function slugifyInstanceName(name: string): string {
    return (name || "")
        .trim()
        .toLowerCase()
        .replace(/\s+/g, "-")
        .replace(/[^a-z0-9-_]/g, "");
}

/**
 * Encode `0..23` as a single base24 character: digits 0-9 then
 * lowercase letters a-n. This compresses the time component into
 * one char so the full stamp fits in 5.
 */
function hourToBase24Char(h: number): string {
    if (h < 0 || h > 23) {
        throw new RangeError(`hour out of range: ${h}`);
    }
    return h < 10 ? String(h) : String.fromCharCode("a".charCodeAt(0) + (h - 10));
}

/**
 * Format a Date as `MMDDh` in the caller's local time:
 *   MM = 2-digit month (01-12)
 *   DD = 2-digit day (01-31)
 *   h  = base24 hour char (0-9 then a-n for 10-23)
 *
 * Total: 5 chars exactly. Two launches within the same hour produce
 * the same slug; the launch flow resolves the collision server-side
 * by calling `WriteAgentConfigCommand` with `auto_allocate: true`,
 * which appends `-N` (1, 2, …) until a free slot is found.
 *
 * Year is omitted because per-version isolation already separates
 * runs across releases; within a version, month+day+hour gives users
 * enough context to recognize their own folders on disk without
 * burning chars on the year prefix.
 */
function formatLocalStamp(d: Date): string {
    const pad2 = (n: number) => String(n).padStart(2, "0");
    return `${pad2(d.getMonth() + 1)}${pad2(d.getDate())}${hourToBase24Char(d.getHours())}`;
}

/**
 * Build the per-instance slug for a given name at the given time.
 * `<slug>-<stamp>` — stable, filesystem-safe, and 5-char date.
 *
 * Collision resolution happens at launch time on the server (see
 * the module docstring). Callers don't need to pass this through any
 * additional helper.
 */
export function buildInstanceSlug(name: string, at: Date = new Date()): string {
    return `${slugifyInstanceName(name)}-${formatLocalStamp(at)}`;
}
