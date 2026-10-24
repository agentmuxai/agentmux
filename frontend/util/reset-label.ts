// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** "Sun 7:00 PM": when an agent's usage limit lifts, in this machine's time zone. */
export function resetLabel(ms: number): string {
    return new Date(ms).toLocaleString(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" });
}
