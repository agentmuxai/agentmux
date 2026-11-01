// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A fork's copy of its parent's transcript, signed for the fork
//! (docs/specs/SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md §6.1, "A fork").
//!
//! A record's signature binds its owner (`origin_sig`), so the lines a fork copies from its parent's
//! zone do not verify for the fork, and the human's own earlier messages would show in the container.
//! After the copy, each user line that verifies for the parent gets the signature for the fork. A line
//! that does not verify is left as it is, so a forged line is never made good by forking. The new
//! signature has the old one's length, so a line keeps its size and the receive-time sidecar's offsets
//! stay right.

use crate::backend::storage::filestore::FileStore;

use super::origin_sig::{resign_line, SIG_LEN};

/// How much of the transcript is read at a time: lines are handled whole, so a line longer than this
/// widens the read.
const CHUNK: i64 = 4 * 1024 * 1024;

/// Sign for `to_owner` the lines of `file` in `zone` that verify for `from_owner`; how many it signed.
pub fn resign_zone(gfs: &FileStore, zone: &str, file: &str, from_owner: &str, to_owner: &str) -> Result<u64, String> {
    let Some(stat) = gfs.stat(zone, file).map_err(|e| format!("stat transcript: {e}"))? else { return Ok(0) };
    let (mut pos, mut signed, mut window) = (0i64, 0u64, CHUNK);
    while pos < stat.size {
        let (at, data) = gfs.read_at(zone, file, pos, window).map_err(|e| format!("read transcript: {e}"))?;
        if data.is_empty() {
            break;
        }
        // Whole lines only; the last one is cut by the window unless the file ends there.
        let end = if at + data.len() as i64 >= stat.size { data.len() } else { data.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1) };
        if end == 0 {
            window = window.saturating_mul(2);
            continue;
        }
        window = CHUNK;
        let mut line_start = 0usize;
        while line_start < end {
            let line_end = data[line_start..end].iter().position(|b| *b == b'\n').map_or(end, |i| line_start + i + 1);
            if let Some((sig_at, new)) = resign_line(from_owner, to_owner, &data[line_start..line_end]) {
                debug_assert_eq!(new.len(), SIG_LEN);
                gfs.write_at(zone, file, at + (line_start + sig_at) as i64, new.as_bytes()).map_err(|e| format!("sign a copied line: {e}"))?;
                signed += 1;
            }
            line_start = line_end;
        }
        pos = at + end as i64;
    }
    Ok(signed)
}

#[cfg(test)]
#[path = "origin_resign_tests.rs"]
mod tests;
