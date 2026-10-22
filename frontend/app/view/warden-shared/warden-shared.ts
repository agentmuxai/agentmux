// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Small helpers shared verbatim across every Warden rail section (Host,
// LAN, Audit, Supervisor) — split out of the original monolithic
// warden.tsx rather than duplicated per-manager.

import { createEffect, onCleanup } from "solid-js";

/** Poll interval every Warden section's own refresh loop uses. */
export const WARDEN_REFRESH_MS = 5_000;

/**
 * Runs `refresh` now and every WARDEN_REFRESH_MS while `visible()` is true, and not while it's
 * false: a Warden section stays mounted when another is shown, so its poll would otherwise keep
 * running unseen. Becoming visible again refreshes at once, so a section shows current data as
 * soon as it's switched to. Call inside a component (it registers its own cleanup).
 */
export function pollWhileVisible(refresh: () => void, visible: () => boolean = () => true): void {
    createEffect(() => {
        if (!visible()) return;
        refresh();
        const timer = window.setInterval(refresh, WARDEN_REFRESH_MS);
        onCleanup(() => window.clearInterval(timer));
    });
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
