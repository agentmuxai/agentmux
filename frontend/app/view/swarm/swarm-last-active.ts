// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { formatExactTime, formatTimeAgo } from "@/util/format-time";

/**
 * Block meta the agent's output stream stamps (Unix ms of its most recent
 * output line, flushed about once a second; `session_stats.rs`). It is the
 * pane's "last active": the same value the agent pane's session stats show.
 */
const LAST_ACTIVITY_META = "session:last_activity_ms";

/** When the agent last produced output, or 0 when it never has (or the pane has no stats yet). */
export function lastActiveMs(meta: Record<string, unknown> | null | undefined): number {
    const v = meta?.[LAST_ACTIVITY_META];
    return typeof v === "number" && Number.isFinite(v) && v > 0 ? v : 0;
}

/** `just now`, `3m ago`, `2h ago`, `1d ago`; empty when there is nothing to say. */
export function lastActiveLabel(ms: number, now: number): string {
    return ms > 0 ? formatTimeAgo(ms, now) : "";
}

/** The row's tooltip: the exact time of that last output. */
export function lastActiveTitle(ms: number): string {
    return ms > 0 ? `Last active ${formatExactTime(ms)}` : "";
}
