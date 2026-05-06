// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared tear-off helper: try the warm pool first, fall back to the
 * cold-path `openWindowAtPosition` only on rejection. Used by every
 * platform-specific `CrossWindowDragMonitor` variant
 * (win32 / darwin / linux).
 *
 * `tabbar.tsx::performTabTearOff` does NOT use this helper — its
 * tear-off pipeline tracks `coldPathFailed` for F1.B orphan-workspace
 * cleanup safety (codex P1 round-3 #624) and pairs the open with an
 * SC_MOVE handshake. Both flows still try-pool-first, just with
 * different surrounding logic.
 *
 * Pool path is ~0ms first paint; cold path is 150–300ms and goes
 * through `create_isolated_request_context` whose stability issues
 * have been observed to destabilize the source window's renderer
 * post-tearoff. Spec: `docs/specs/SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md`.
 */

import type { getApi } from "@/store/global";
import { Logger } from "@/util/logger";

type Api = ReturnType<typeof getApi>;

/**
 * Open a tear-off destination window at `(screenX, screenY)`,
 * preferring the pre-warmed pool. Falls back to cold-path only when
 * `tearOffPoolPromote` rejects (e.g. pool exhausted, host refuses).
 */
export async function openTearOffWindow(
    api: Api,
    newWsId: string,
    screenX: number,
    screenY: number,
): Promise<void> {
    try {
        await api.tearOffPoolPromote(newWsId, screenX, screenY);
    } catch (poolErr) {
        Logger.warn("dnd:cross", "pool promote failed, cold-pathing", {
            error: String(poolErr),
        });
        await api.openWindowAtPosition(screenX, screenY, newWsId);
    }
}
