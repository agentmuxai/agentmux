// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! FileStore IJson (incremental JSON) operations.

#![allow(dead_code)]

use super::core::FileStore;
use super::types::FileMeta;
use crate::backend::storage::error::StoreError;

impl FileStore {
    /// IJson metadata key: number of commands since last compaction.
    const IJSON_NUM_COMMANDS: &'static str = "ijson:numcmds";
    /// IJson metadata key: incremental bytes since last compaction.
    const IJSON_INC_BYTES: &'static str = "ijson:incbytes";

    /// Compaction threshold: high command count.
    const IJSON_HIGH_COMMANDS: i64 = 100;
    /// Compaction threshold: high ratio (incremental/file >= 3x).
    const IJSON_HIGH_RATIO: f64 = 3.0;
    /// Compaction threshold: low command count.
    const IJSON_LOW_COMMANDS: i64 = 10;
    /// Compaction threshold: low ratio (incremental/file >= 1x).
    const IJSON_LOW_RATIO: f64 = 1.0;

    /// Append an IJson command to a file. Triggers compaction if thresholds are exceeded.
    pub fn append_ijson(
        &self,
        zone_id: &str,
        name: &str,
        command: &serde_json::Value,
    ) -> Result<(), StoreError> {
        let file = self.stat(zone_id, name)?.ok_or(StoreError::NotFound)?;
        if !file.opts.ijson {
            return Err(StoreError::Other("file is not ijson".to_string()));
        }

        let cmd_bytes = serde_json::to_string(command)?;
        let data = format!("{}\n", cmd_bytes);
        self.append_data(zone_id, name, data.as_bytes())?;

        // Update IJson metadata counters
        let file = self.stat(zone_id, name)?.ok_or(StoreError::NotFound)?;
        let mut meta = file.meta.clone();

        let num_cmds = meta
            .get(Self::IJSON_NUM_COMMANDS)
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            + 1;
        let inc_bytes = meta
            .get(Self::IJSON_INC_BYTES)
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            + data.len() as i64;

        meta.insert(
            Self::IJSON_NUM_COMMANDS.to_string(),
            serde_json::json!(num_cmds),
        );
        meta.insert(
            Self::IJSON_INC_BYTES.to_string(),
            serde_json::json!(inc_bytes),
        );
        self.write_meta(zone_id, name, meta, false)?;

        // Check compaction thresholds
        let file_size = file.size.max(1); // avoid division by zero
        let ratio = inc_bytes as f64 / file_size as f64;

        let should_compact = num_cmds > Self::IJSON_HIGH_COMMANDS
            || ratio >= Self::IJSON_HIGH_RATIO
            || (num_cmds > Self::IJSON_LOW_COMMANDS && ratio >= Self::IJSON_LOW_RATIO);

        if should_compact {
            let _ = self.compact_ijson(zone_id, name);
        }

        Ok(())
    }

    /// Compact an IJson file: apply all incremental commands to build compacted state,
    /// then replace file contents with the compacted result.
    pub fn compact_ijson(
        &self,
        zone_id: &str,
        name: &str,
    ) -> Result<(), StoreError> {
        // Read full file
        let data = self
            .read_file(zone_id, name)?
            .ok_or(StoreError::NotFound)?;

        let content = String::from_utf8(data)
            .map_err(|e| StoreError::Other(format!("invalid utf-8 in ijson file: {}", e)))?;

        // Use the ijson module's compact function
        let compacted = crate::backend::ijson::compact_ijson(&content)
            .map_err(|e| StoreError::Other(format!("ijson compact error: {}", e)))?;

        // The compacted result is a single JSON command (set at root).
        // Write it as the new file content.
        let compacted_with_newline = format!("{}\n", compacted);
        self.write_file(zone_id, name, compacted_with_newline.as_bytes())?;

        // Reset IJson counters
        let mut meta = FileMeta::new();
        meta.insert(
            Self::IJSON_NUM_COMMANDS.to_string(),
            serde_json::json!(0),
        );
        meta.insert(
            Self::IJSON_INC_BYTES.to_string(),
            serde_json::json!(0),
        );
        self.write_meta(zone_id, name, meta, true)?;

        Ok(())
    }
}
