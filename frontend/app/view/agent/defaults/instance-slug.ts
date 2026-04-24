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

/**
 * Format a Date as `YYYYMMDD-HHMMSS-mmm` in the caller's local time.
 * The millisecond suffix makes the stamp unique across realistic
 * concurrent launches — two clicks in the same millisecond are not
 * a failure mode we optimise for. Codex flagged sub-second collisions
 * on PR #504 (https://github.com/agentmuxai/agentmux/pull/504); this
 * form closes that gap without reaching for UUIDs.
 */
export function formatLocalStamp(d: Date): string {
    const pad = (n: number, w = 2) => String(n).padStart(w, "0");
    return (
        `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}` +
        `-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}` +
        `-${pad(d.getMilliseconds(), 3)}`
    );
}

/**
 * Build the per-instance slug for a given name at the given time.
 * `<slug>-<stamp>` — stable, filesystem-safe, and unique at 1ms
 * granularity.
 */
export function buildInstanceSlug(name: string, at: Date = new Date()): string {
    return `${slugifyInstanceName(name)}-${formatLocalStamp(at)}`;
}
