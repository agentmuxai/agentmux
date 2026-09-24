// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! FileStore: file storage with write-through cache + background flusher.
//! Port of Go's pkg/filestore/blockstore.go, blockstore_cache.go, blockstore_dbops.go.
//!
//! - Separate SQLite DB from Store (matching Go).
//! - 64KB parts for efficient partial reads/writes.
//! - Write-through cache with periodic flush (5s default).
//! - Background flusher via `tokio::spawn` + `tokio::time::interval`.

mod cache;
mod core;
mod counter;
mod ijson;
mod lines;
mod offset_ops;
mod replace;
mod types;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_atomic;
#[cfg(test)]
mod tests_counter;
#[cfg(test)]
mod tests_replace;

#[allow(unused_imports)]
pub use core::{FileStore, DEFAULT_FLUSH_SECS, MAX_CACHE_BYTES};
pub use types::{FileMeta, FileOpts, MuxFile};
pub(crate) use lines::is_blank_line;
#[allow(unused_imports)] // consumed by the transcript writers in 5a-3
pub use counter::{AppendPos, Counted, CountedAppend, LineState};
