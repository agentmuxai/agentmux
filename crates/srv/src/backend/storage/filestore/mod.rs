// Copyright 2025, Command Line Inc.
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
mod checkpointer;
mod copy;
mod core;
mod counter;
mod deltas;
mod frames;
mod ijson;
mod lines;
mod off_worker;
mod offset_ops;
mod parts;
pub mod reclaim;
pub mod repair;
mod replace;
mod trim;
mod truncate_owner;
mod types;
mod zone_txn;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_append_reference;
#[cfg(test)]
mod tests_off_worker;
#[cfg(test)]
mod tests_atomic;
#[cfg(test)]
mod tests_copy;
#[cfg(test)]
mod tests_counter;
#[cfg(test)]
mod tests_lost_parts;
#[cfg(test)]
mod tests_replace;
#[cfg(test)]
mod tests_stamp_throttle;
#[cfg(test)]
mod tests_trim;
#[cfg(test)]
mod tests_readers;
#[cfg(test)]
mod tests_checkpoint;
#[cfg(test)]
mod tests_global_version;
#[cfg(test)]
mod tests_reclaim;
#[cfg(test)]
mod tests_repair;
#[cfg(test)]
mod tests_tail_slack;

#[allow(unused_imports)]
pub use core::{FileStore, DEFAULT_FLUSH_SECS, MAX_CACHE_BYTES};
pub use truncate_owner::TruncateOwner;
pub use types::{FileMeta, FileOpts, MuxFile};
#[allow(unused_imports)] // the memory record (SPEC_MEMORY_FOLLOWS_THE_AGENT M3)
pub use zone_txn::ZoneTxn;
pub(crate) use lines::is_blank_line;
#[cfg(test)]
pub(crate) use parts::strip_slack as strip_slack_for_test;
pub(crate) use trim::{append_log_cost, first_live_part};
pub(crate) use types::RETAINED_FROM_KEY;
pub(crate) use frames::{FrameOutcome, CODEC_DELTAS, CODEC_ZSTD, FRAME_PARTS};
#[allow(unused_imports)] // consumed by the transcript writers in 5a-3
pub use counter::{normalized_records, AppendPos, Counted, CountedAppend, LineState, TailState};
#[allow(unused_imports)]
pub use replace::{DerivedSnapshot, DerivedView, SnapshotFile, SnapshotReader};
