// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One `install.check` per provider CLI, shared by every agent picker.
 *
 * The answer depends only on the provider's CLI (`providerId`, `cliCommand`,
 * `npmPackage`), never on which agent asked, so N agents on the same
 * provider, in any number of pickers, share one round trip. A picker used
 * to issue one check per agent, and re-issue it for every agent still
 * pending each time an answer came back: ~700 calls per new window tab
 * with ~37 agents, which kept the page's main thread busy for ~400 ms
 * (docs/analysis/ANALYSIS_NEW_WINDOW_TAB_LATENCY_2026_09_30.md).
 *
 * A result is reused for `TTL_MS`; an in-flight check is shared. The
 * install flow calls `invalidateInstallCheck` once it has installed a CLI,
 * and a failed check is never cached.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { InstallCheckReq } from "@/types/rpc/InstallCheckReq";
import type { InstallCheckResult } from "@/types/rpc/InstallCheckResult";

const TTL_MS = 30_000;

type Entry = { startedAt: number; result: Promise<InstallCheckResult> };
const entries = new Map<string, Entry>();

const keyOf = (req: InstallCheckReq) => `${req.providerId}\u0000${req.cliCommand}\u0000${req.npmPackage ?? ""}`;

export function checkProviderInstalled(req: InstallCheckReq, now: () => number = Date.now): Promise<InstallCheckResult> {
    const key = keyOf(req);
    const hit = entries.get(key);
    if (hit && now() - hit.startedAt < TTL_MS) return hit.result;
    const entry: Entry = { startedAt: now(), result: RpcApi.InstallCheckCommand(TabRpcClient, req) };
    entries.set(key, entry);
    entry.result.catch(() => {
        if (entries.get(key) === entry) entries.delete(key);
    });
    return entry.result;
}

/** Forget cached answers for `providerId` (every CLI of it), or for all providers. */
export function invalidateInstallCheck(providerId?: string): void {
    if (providerId == null) {
        entries.clear();
        return;
    }
    for (const key of [...entries.keys()]) {
        if (key.startsWith(`${providerId}\u0000`)) entries.delete(key);
    }
}
