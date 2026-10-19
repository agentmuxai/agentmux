// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keeps a blocking store call from pinning a runtime worker.
//! docs/specs/SPEC_FILESTORE_STORE_CALLS_OFF_WORKERS_2026_10_19.md
//!
//! The store's calls are synchronous and many are made from async handlers. All
//! of them wait for one connection, and the holder can be stuck in a disk stall
//! for seconds (#5141). Each waiter used to sit on a tokio worker: with eight
//! workers and eight callers queued, every worker was blocked and nothing else
//! in srv ran, however unrelated.

/// Run `f` so that, on a multi-thread runtime's worker, the worker's other tasks
/// move to another thread while `f` blocks (`tokio::task::block_in_place`).
/// Anywhere else (a plain thread, a blocking-pool thread, a current-thread
/// runtime, a test without a runtime) it just calls `f`.
pub(super) fn off_worker<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    }
}
