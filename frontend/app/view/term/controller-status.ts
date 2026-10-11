// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { BlockControllerRuntimeStatus } from "@/types/rpc/BlockControllerRuntimeStatus";

/** Whether `next` is newer than `cur` for a pane's controller status.
 *  Statuses from two spawns compare by spawn time: a restart replaces the
 *  controller, whose versions start again, so its "running" lost to the old
 *  one's "done", and the pane kept restarting on every Enter (#4513); and
 *  the replaced controller's own late "done" must not win either. Within
 *  one spawn, or without a spawn time (before a spawn), the version decides. */
export function isNewerControllerStatus(
    cur: BlockControllerRuntimeStatus | null | undefined,
    next: BlockControllerRuntimeStatus
): boolean {
    if (cur == null) return true;
    const [a, b] = [cur.spawn_ts_ms ?? 0, next.spawn_ts_ms ?? 0];
    if (a && b && a !== b) return b > a;
    return next.version > cur.version;
}
