// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! FileStore offset-based read/write operations.


use std::collections::HashMap;

use rusqlite::params;

use super::core::{FileStore, PART_DATA_SIZE};
use crate::backend::storage::error::StoreError;

impl FileStore {
    /// Write data at a specific offset.
    /// The offset must be <= current file size.
    pub fn write_at(
        &self,
        zone_id: &str,
        name: &str,
        offset: i64,
        data: &[u8],
    ) -> Result<(), StoreError> {
        if data.is_empty() {
            return Ok(());
        }

        let key = (zone_id.to_string(), name.to_string());
        let now = Self::now_ms();

        let file = self.stat(zone_id, name)?.ok_or(StoreError::NotFound)?;
        if offset > file.size {
            return Err(StoreError::Other(format!(
                "offset {} exceeds file size {}",
                offset, file.size
            )));
        }

        let new_size = std::cmp::max(file.size, offset + data.len() as i64);
        let pds = PART_DATA_SIZE as i64;

        // Handle circular file data truncation
        let (actual_offset, actual_data) = if file.opts.circular && file.opts.maxsize > 0 {
            let start_cir_offset = new_size - file.opts.maxsize;
            if start_cir_offset > 0 {
                let end = offset + data.len() as i64;
                if end <= start_cir_offset {
                    // Entire write is before the circular window — no-op
                    return Ok(());
                }
                if offset < start_cir_offset {
                    let skip = (start_cir_offset - offset) as usize;
                    (start_cir_offset, &data[skip..])
                } else {
                    (offset, data)
                }
            } else {
                (offset, data)
            }
        } else {
            (offset, data)
        };

        // Compute affected parts
        let start_part = (actual_offset / pds) as i32;
        let end_part = ((actual_offset + actual_data.len() as i64 - 1) / pds) as i32;

        let conn = self.conn.lock().unwrap();
        let mut data_pos = 0usize;

        for part_idx in start_part..=end_part {
            let part_start = part_idx as i64 * pds;
            let offset_in_part = if part_idx == start_part {
                (actual_offset - part_start) as usize
            } else {
                0
            };

            // Load existing part if needed
            let existing: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3",
                    params![zone_id, name, part_idx],
                    |row| row.get(0),
                )
                .ok();

            let mut part_data = existing.unwrap_or_default();
            // Ensure part is large enough
            if part_data.len() < offset_in_part {
                part_data.resize(offset_in_part, 0);
            }

            // Copy data into part
            let remaining = actual_data.len() - data_pos;
            let space = PART_DATA_SIZE - offset_in_part;
            let to_copy = remaining.min(space);

            if offset_in_part < part_data.len() {
                // Overwrite existing bytes
                let overwrite_end = (offset_in_part + to_copy).min(part_data.len());
                let overwrite_len = overwrite_end - offset_in_part;
                part_data[offset_in_part..offset_in_part + overwrite_len]
                    .copy_from_slice(&actual_data[data_pos..data_pos + overwrite_len]);
                if to_copy > overwrite_len {
                    part_data.extend_from_slice(
                        &actual_data[data_pos + overwrite_len..data_pos + to_copy],
                    );
                }
            } else {
                part_data.extend_from_slice(&actual_data[data_pos..data_pos + to_copy]);
            }

            conn.execute(
                "REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                params![zone_id, name, part_idx, part_data],
            )?;
            data_pos += to_copy;
        }

        // Update file size
        conn.execute(
            "UPDATE db_wave_file SET size = ?1, modts = ?2 WHERE zoneid = ?3 AND name = ?4",
            params![new_size, now, zone_id, name],
        )?;
        drop(conn);

        // Update cache
        let mut cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get_mut(&key) {
            if let Some(ref mut f) = entry.file {
                f.size = new_size;
                f.modts = now;
            }
            // Invalidate cached data entries for affected parts
            for part_idx in start_part..=end_part {
                entry.data_entries.remove(&part_idx);
            }
        }

        Ok(())
    }

    /// Read data at a specific offset and size.
    /// For circular files, adjusts offset if it falls before valid data range.
    /// Returns (adjusted_offset, data).
    pub fn read_at(
        &self,
        zone_id: &str,
        name: &str,
        offset: i64,
        size: i64,
    ) -> Result<(i64, Vec<u8>), StoreError> {
        let file = self
            .stat(zone_id, name)?
            .ok_or(StoreError::NotFound)?;

        if file.size == 0 {
            return Ok((0, Vec::new()));
        }

        let data_len = file.data_length();
        let data_start = file.data_start_idx();

        // Adjust offset for circular files
        let mut actual_offset = offset;
        let mut actual_size = if size == 0 { data_len } else { size };

        if file.opts.circular && file.opts.maxsize > 0 {
            if actual_offset < data_start {
                let skip = data_start - actual_offset;
                actual_offset = data_start;
                actual_size -= skip;
            }
            if actual_size <= 0 {
                return Ok((data_start, Vec::new()));
            }
        }

        // Clamp to available data
        if actual_offset >= file.size {
            return Ok((actual_offset, Vec::new()));
        }
        let available = file.size - actual_offset;
        actual_size = actual_size.min(available);

        if actual_size <= 0 {
            return Ok((actual_offset, Vec::new()));
        }

        let pds = PART_DATA_SIZE as i64;
        let start_part = (actual_offset / pds) as i32;
        let end_part = ((actual_offset + actual_size - 1) / pds) as i32;

        // Load parts from DB
        let conn = self.conn.lock().unwrap();
        let mut parts_map: HashMap<i32, Vec<u8>> = HashMap::new();
        for part_idx in start_part..=end_part {
            if let Ok(data) = conn.query_row(
                "SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3",
                params![zone_id, name, part_idx],
                |row| row.get::<_, Vec<u8>>(0),
            ) {
                parts_map.insert(part_idx, data);
            }
        }
        drop(conn);

        // Assemble result
        let mut result = Vec::with_capacity(actual_size as usize);
        for part_idx in start_part..=end_part {
            if let Some(part_data) = parts_map.get(&part_idx) {
                let part_start = part_idx as i64 * pds;
                let skip = if part_start < actual_offset {
                    (actual_offset - part_start) as usize
                } else {
                    0
                };
                let remaining = actual_size as usize - result.len();
                let take = remaining.min(part_data.len().saturating_sub(skip));
                if take > 0 {
                    result.extend_from_slice(&part_data[skip..skip + take]);
                }
            }
        }

        Ok((actual_offset, result))
    }
}
