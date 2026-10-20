// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** When something happened, `ts` in unix millis: relative for today, a date before that. */
export function formatWhen(ts: number, now = Date.now()): string {
    if (!ts) return "never";
    const d = Math.max(0, now - ts);
    if (d < 60_000) return "just now";
    if (d < 3_600_000) return `${Math.floor(d / 60_000)} min ago`;
    if (d < 86_400_000) return `${Math.floor(d / 3_600_000)} h ago`;
    return new Date(ts).toLocaleDateString();
}
