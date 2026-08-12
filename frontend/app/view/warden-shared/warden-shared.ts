// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Small helpers shared verbatim across every Warden rail section (Host,
// LAN, Audit, Supervisor) — split out of the original monolithic
// warden.tsx rather than duplicated per-manager.

import { getApi } from "@/store/global";

/** Poll interval every Warden section's own refresh loop uses. */
export const WARDEN_REFRESH_MS = 5_000;

export function authedHeaders(): Record<string, string> {
    const headers: Record<string, string> = {};
    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) headers["X-AuthKey"] = authKey;
    }
    return headers;
}

/** `ts` is a unix-millis timestamp from the Rust backend. */
export function ageMs(ts: number, now: number): number {
    return Math.max(0, now - ts);
}

export function formatAge(ms: number): string {
    if (ms < 60_000) return `${Math.floor(ms / 1000)}s`;
    if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m`;
    return `${Math.floor(ms / 3_600_000)}h`;
}
