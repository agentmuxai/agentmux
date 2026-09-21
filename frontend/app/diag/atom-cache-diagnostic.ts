// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! atom-cache-diagnostic — periodic, read-only visibility into three
//! related caches implicated in a 2026-09-20 latency/growing-page-file
//! report: `mos.ts`'s `muxObjectValueCache` (pinned by #3454's refCount
//! change), `block-atom-cache.ts`'s per-block `createRoot`-wrapped memos
//! (the thing that holds those pins open), and the block-component
//! registry's dormant/keep-alive count (what keeps a block's atom-cache
//! entries alive instead of being torn down on close).
//!
//! Static analysis (see the three modules' own diagnostic-getter doc
//! comments) found a real, narrower-than-first-suspected mechanism: a
//! term/agent tab that's stacked into a pane and left there — dormant, never
//! closed — pins its cache entries indefinitely instead of being reclaimed.
//! This module does NOT change that behavior. It only reports numbers, on a
//! timer, so a real recurrence can be confirmed (or ruled out) from logs
//! instead of requiring a live DevTools heap-snapshot session — the same
//! "instrument first" sequence `agentmux-srv`'s own `mem_attribution`
//! (`sysinfo.rs`) already follows, and the same one PR #3471 used for the
//! process-tracker/orphan-reaper subsystems the same night this was written.
//!
//! Goes through frontend's `[fe]` log pipe → host log (see
//! `replace-child-diagnostic.ts` and `app-init.ts`'s `[wave-title]` for the
//! same convention); tail with `muxlog host '\[fe\] \[atom-cache-diag\]'`.
//! A rising `pinnedEntries`/`dormantCount` that never comes back down as
//! panes go dormant/close, over a long session, would confirm the mechanism
//! is live rather than theoretical.

import { getMuxObjectCacheStats } from "@/app/store/mos";
import { getBlockAtomCacheStats } from "@/app/store/block-atom-cache";
import { getBlockComponentRegistryStats } from "@/app/store/block-component-registry";

const REPORT_INTERVAL_MS = 30000;

/** Exported for tests — gathers the snapshot without logging it. */
export function collectAtomCacheDiagnostics() {
    return {
        muxObjectCache: getMuxObjectCacheStats(),
        blockAtomCache: getBlockAtomCacheStats(),
        registry: getBlockComponentRegistryStats(),
    };
}

/**
 * Exported for tests — the log line, split from the timer that drives it.
 *
 * Deliberately `console.log`, not `console.debug`: `log-pipe.ts` forwards
 * `debug` to the Rust host as `tracing::debug!`, and `logging.rs`'s default
 * `EnvFilter` is `"info"` — a `debug`-level line is silently dropped from
 * the host log unless `RUST_LOG`/equivalent is raised, with no error
 * anywhere in the chain (confirmed live: the very first shipped version of
 * this file used `console.debug` and never once appeared in a real host log
 * despite firing correctly every 30s, verified via a direct CDP
 * `Runtime.evaluate` check against the running window). `console.log` maps
 * to `tracing::info!` (`backend.rs`'s `fe_log_structured`), matching
 * `mem_attribution`'s own level choice on the Rust side.
 */
export function logAtomCacheDiagnostics() {
    console.log("[atom-cache-diag]", JSON.stringify(collectAtomCacheDiagnostics()));
}

setInterval(logAtomCacheDiagnostics, REPORT_INTERVAL_MS);
