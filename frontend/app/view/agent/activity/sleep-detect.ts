// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Recognizes a Bash command that is *nothing but* a wait, so the dock can
 * promote it immediately (instead of after `TOOL_PROMOTION_MS`) and show a
 * real countdown instead of a blind elapsed timer.
 *
 * ## Why this is deliberately narrow
 *
 * `REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md` §4.2 rejected
 * text matching as the *classifier* — correctly. Measured against 7,761
 * foreground Bash calls from real transcripts, the obvious rule ("command
 * starts with `sleep`") matches 270 calls of which **204 (76%) are not waits
 * at all** — they're `sleep 90; tail -30 <log>`, `sleep 60; ls <dir>`,
 * `sleep 30; cat <file>`: poll-then-inspect. Promoting on that basis would be
 * wrong three times out of four.
 *
 * What that rejection over-shot is the case where the command is a wait and
 * **nothing else**. `sleep 300` has no second clause to be wrong about, so
 * matching it has no false-positive surface *by construction* — the risk the
 * report identified lives entirely in the part after the `&&`/`;`, and this
 * matcher refuses anything that has one. Same 7,761 calls: 66 match, every
 * one a genuine wait, median 61s.
 *
 * Duration-based promotion remains the classifier for everything else,
 * exactly as that report specifies. This is the report's own "phase 1a, cheap
 * UX polish" — layered on top of the duration rule, never replacing it. The
 * 204 compound sleeps this ignores all ran 28-100s, so the duration rule
 * already catches every one of them; skipping them here costs nothing.
 */

/**
 * Below this, a bare sleep is a micro-delay, not something worth a dock row —
 * `sleep 1` between two commands would otherwise occupy the dock for its own
 * duration plus the full `RETENTION_MS.done` window, which is strictly more
 * noise than signal. Such commands keep the ordinary duration behaviour (i.e.
 * they never promote, because they finish long before the threshold).
 */
export const SLEEP_IMMEDIATE_MIN_MS = 5_000;

/**
 * The whole command, and only a wait:
 *   `sleep 300` · `sleep 300;` · `sleep 0.5` · `sleep 5m` · `timeout 60 sleep 60`
 *
 * Anything with a second clause (`&&`, `;` followed by work, `|`, a newline)
 * fails to match and falls through to duration promotion. So does a bare
 * `sleep` with no argument, and GNU's multi-arg `sleep 1 2` form (rare, and
 * summing it is not worth the surface).
 */
const WHOLE_COMMAND_SLEEP = /^\s*(?:timeout\s+\d+(?:\.\d+)?[smhd]?\s+)?sleep\s+(\d+(?:\.\d+)?)([smhd]?)\s*;?\s*$/i;

const UNIT_MS: Record<string, number> = { "": 1_000, s: 1_000, m: 60_000, h: 3_600_000, d: 86_400_000 };

/**
 * Milliseconds this command will sleep for, or `null` when it isn't a
 * whole-command sleep (the overwhelmingly common case — 99.15% of real Bash
 * calls). Callers treat `null` as "no special handling, use the duration
 * rule".
 *
 * Returns `null` below [`SLEEP_IMMEDIATE_MIN_MS`] too, so a single check
 * answers both "is this a pure wait" and "is it worth showing" — no caller
 * has to remember to apply the floor itself.
 */
export function wholeCommandSleepMs(command: string | undefined): number | null {
    if (!command) return null;
    const m = WHOLE_COMMAND_SLEEP.exec(command);
    if (!m) return null;
    const value = Number.parseFloat(m[1]);
    if (!Number.isFinite(value)) return null;
    const ms = value * (UNIT_MS[m[2].toLowerCase()] ?? 1_000);
    return ms >= SLEEP_IMMEDIATE_MIN_MS ? ms : null;
}
