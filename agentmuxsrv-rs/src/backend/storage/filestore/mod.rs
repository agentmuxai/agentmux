// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! FileStore: file storage with write-through cache + background flusher.
//! Port of Go's pkg/filestore/blockstore.go, blockstore_cache.go, blockstore_dbops.go.
//!
//! - Separate SQLite DB from WaveStore (matching Go).
//! - 64KB parts for efficient partial reads/writes.
//! - Write-through cache with periodic flush (5s default).
//! - Background flusher via `tokio::spawn` + `tokio::time::interval`.

mod cache;
mod core;
mod ijson;
mod offset_ops;
mod types;
#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use core::{FileStore, DEFAULT_FLUSH_SECS};
pub use types::{FileMeta, FileOpts, WaveFile};
