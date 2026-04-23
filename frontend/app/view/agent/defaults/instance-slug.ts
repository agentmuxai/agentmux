// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared slug + timestamp helpers for building per-instance working
 * directories. Kept as a standalone module so the AgentLaunchModal
 * (for preview) and the AgentViewModel (for actual launch) agree on
 * the format without importing from each other's files.
 *
 * Format: `<slug(name)>-<YYYYMMDD-HHMMSS>`, local time.
 * See docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md §7.
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

/** Format a Date as `YYYYMMDD-HHMMSS` in the caller's local time. */
export function formatLocalStamp(d: Date): string {
    const pad = (n: number) => String(n).padStart(2, "0");
    return (
        `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}` +
        `-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`
    );
}

/**
 * Build the per-instance slug for a given name at the given time.
 * `<slug>-<stamp>` — stable, filesystem-safe, and unique at 1s
 * granularity. Sub-second collisions are resolved by the backend
 * with a `-1`, `-2`, … suffix.
 */
export function buildInstanceSlug(name: string, at: Date = new Date()): string {
    return `${slugifyInstanceName(name)}-${formatLocalStamp(at)}`;
}
