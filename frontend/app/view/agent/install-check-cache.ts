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

/** `answer` is what callers get: `rpc`'s result while it is still current. */
type Entry = { startedAt: number; rpc: Promise<InstallCheckResult>; answer: Promise<InstallCheckResult> };
const entries = new Map<string, Entry>();

const keyOf = (req: InstallCheckReq) => `${req.providerId}\u0000${req.cliCommand}\u0000${req.npmPackage ?? ""}`;

export function checkProviderInstalled(req: InstallCheckReq, now: () => number = Date.now): Promise<InstallCheckResult> {
    const key = keyOf(req);
    const hit = entries.get(key);
    if (hit && now() - hit.startedAt < TTL_MS) return hit.answer;
    const rpc = RpcApi.InstallCheckCommand(TabRpcClient, req);
    const entry = { startedAt: now(), rpc } as Entry;
    // An answer forgotten while in flight (an install finished, a refresh)
    // is stale: the caller gets the newer check's answer instead, asked now
    // if none started, so a slow old answer can't overwrite a newer one.
    entry.answer = rpc.then((result) => {
        const current = entries.get(key);
        if (current === entry) return result;
        return current ? current.answer : checkProviderInstalled(req, now);
    });
    entries.set(key, entry);
    rpc.catch(() => {
        if (entries.get(key) === entry) entries.delete(key);
    });
    return entry.answer;
}

/** Told when answers are forgotten: `providerId`, or undefined for all. */
const invalidationListeners = new Set<(providerId: string | undefined) => void>();

/**
 * Runs `cb` whenever cached answers are forgotten, so a mounted consumer (a
 * picker's cards, the Toolchain's rows) asks again instead of keeping the
 * answer it copied: an install in one, or a refresh that finds a CLI, shows in
 * the other. Returns the unsubscribe.
 */
export function onInstallCheckInvalidated(cb: (providerId: string | undefined) => void): () => void {
    invalidationListeners.add(cb);
    return () => invalidationListeners.delete(cb);
}

/** Forget cached answers for `providerId` (every CLI of it), or for all providers. */
export function invalidateInstallCheck(providerId?: string): void {
    if (providerId == null) {
        entries.clear();
    } else {
        for (const key of [...entries.keys()]) {
            if (key.startsWith(`${providerId}\u0000`)) entries.delete(key);
        }
    }
    for (const cb of [...invalidationListeners]) cb(providerId);
}
