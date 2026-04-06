// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Internal cache structs for FileStore.


use std::collections::HashMap;

use super::types::WaveFile;

/// Cache entry for file data parts.
#[derive(Debug, Clone)]
pub(super) struct DataCacheEntry {
    #[allow(dead_code)]
    pub(super) part_idx: i32,
    #[allow(dead_code)]
    pub(super) data: Vec<u8>,
}

/// Cache entry for a file + its data parts.
#[derive(Debug)]
pub(super) struct CacheEntry {
    pub(super) file: Option<WaveFile>,
    #[allow(dead_code)]
    pub(super) data_entries: HashMap<i32, DataCacheEntry>,
    #[allow(dead_code)]
    pub(super) dirty: bool,
    /// Last time this entry was read or written (ms since epoch).
    /// Used for TTL-based eviction of clean entries.
    pub(super) last_access_ms: i64,
}
