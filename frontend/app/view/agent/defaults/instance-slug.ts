// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared slug + timestamp helpers for building per-instance working
 * directories. Kept as a standalone module so the AgentLaunchModal
 * (for preview) and the AgentViewModel (for actual launch) agree on
 * the format without importing from each other's files.
 *
 * Format: `<slug(name)>-MMDDh`, local time.
 *
 * NOTE: Collision resolution (when two launches in the same hour
 * produce identical slugs) is currently NOT implemented end-to-end —
 * the actual launch flow uses `WriteAgentConfigCommand` which writes
 * config into whatever path it's given. Two same-hour launches with
 * the same instance name will share a workdir and overwrite each
 * other's config files. Tracked for follow-up; needs an RPC return-
 * type change (final path) + frontend `cmd:cwd` patch + atomic
 * `create_dir` allocation on the backend.
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
 * Total: 5 chars exactly. Two launches within the same hour collide
 * and require the launch-time `-N` counter (see allocateWorkDir).
 *
 * Year is omitted because per-version isolation already separates
 * runs across releases; within a version, month+day+hour gives users
 * enough context to recognize their own folders on disk without
 * burning chars on the year prefix.
 */
export function formatLocalStamp(d: Date): string {
    const pad2 = (n: number) => String(n).padStart(2, "0");
    return `${pad2(d.getMonth() + 1)}${pad2(d.getDate())}${hourToBase24Char(d.getHours())}`;
}

/**
 * Build the per-instance slug for a given name at the given time.
 * `<slug>-<stamp>` — stable, filesystem-safe, and 5-char date.
 *
 * Collision resolution: callers should pass the result through
 * `allocateWorkDir()` or equivalent at launch time to append `-N`
 * when the directory already exists.
 */
export function buildInstanceSlug(name: string, at: Date = new Date()): string {
    return `${slugifyInstanceName(name)}-${formatLocalStamp(at)}`;
}
