// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Backend uptime: which clock to trust, and how to render it.
 *
 * Extracted from `BackendStatus.tsx` as pure functions for direct unit
 * coverage, same rationale as `disk-volumes.ts` in this directory.
 *
 * Why this exists as its own module: uptime used to be derived entirely
 * from two independent WALL-CLOCK stamps —
 * `backend_started_at` (`chrono::Utc::now()`, stamped once by the host in
 * `agentmux-cef/src/sidecar.rs` when it spawns the sidecar, never
 * re-stamped) and the sysinfo event's `ts` (`SystemTime::now()` on every
 * tick in `agentmux-srv/src/backend/sysinfo.rs`). Subtracting them is only
 * correct while the system clock is monotonic across the backend's whole
 * lifetime, which it is not: an NTP correction, a manual clock set, or a VM
 * resume steps it, and any BACKWARDS step makes the difference negative for
 * the rest of that backend's life.
 *
 * Observed live on 0.55.26: the app started while the machine's clock read
 * 2081-02-05 and the clock was later corrected to 2026-08-29, with no
 * restart in between (one `agentmuxsrv starting` line spanning both eras in
 * the same log file). uptimeSecs sat at about -1.7e9 and the status bar
 * rendered `-59:0-14`, ticking `0-14` -> `0-13` -> `0-12` — a padded minus
 * sign that read as a countdown.
 *
 * The fix is to stop subtracting wall-clock stamps at all: srv now reports
 * `uptime_secs` measured from a monotonic `Instant` captured at its own
 * start, which no clock step can perturb. The wall-clock path is kept only
 * as a clamped fallback for a sysinfo payload that predates that field
 * (e.g. one replayed from the persist ring across an upgrade).
 */

function pad2(n: number): string {
    return n < 10 ? `0${n}` : `${n}`;
}

/**
 * Render a duration in seconds as `m:ss` / `h:mm:ss` / `d:hh:mm:ss`.
 *
 * Negative and non-finite input clamps to zero. That guard is deliberately
 * here rather than only at the call site: this is also the formatter for the
 * crash popover's `backendDeathInfo.uptime_secs`, which arrives from the
 * host's own wall-clock arithmetic and is exposed to the exact same class of
 * clock step.
 */
export function formatUptime(secs: number): string {
    const safe = Number.isFinite(secs) && secs > 0 ? Math.floor(secs) : 0;
    const s = safe % 60;
    const m = Math.floor(safe / 60) % 60;
    const h = Math.floor(safe / 3600) % 24;
    const d = Math.floor(safe / 86400);
    if (d > 0) return `${d}:${pad2(h)}:${pad2(m)}:${pad2(s)}`;
    if (h > 0) return `${h}:${pad2(m)}:${pad2(s)}`;
    return `${m}:${pad2(s)}`;
}

/**
 * Decide the uptime to display from one sysinfo tick.
 *
 * Prefers `reported` — srv's monotonic `uptime_secs`. Falls back to the
 * legacy wall-clock difference (clamped at zero) when that field is absent,
 * and returns `null` when neither source is usable so the caller leaves its
 * previous value alone rather than flashing a zero.
 */
export function resolveUptimeSecs(
    reported: unknown,
    sysinfoTsMs: unknown,
    startedAtMs: number | null,
): number | null {
    if (typeof reported === "number" && Number.isFinite(reported) && reported >= 0) {
        return Math.floor(reported);
    }
    if (typeof sysinfoTsMs === "number" && Number.isFinite(sysinfoTsMs) && startedAtMs != null) {
        return Math.max(0, Math.floor((sysinfoTsMs - startedAtMs) / 1000));
    }
    return null;
}
