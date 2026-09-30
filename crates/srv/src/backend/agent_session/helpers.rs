// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Small shared helpers: monotonic-ish timestamp + FileStore I/O wrappers.


use crate::backend::storage::filestore::{FileMeta, FileOpts, FileStore};

pub(crate) fn now_ms() -> u64 {
    agentmux_common::time::now_ms_u64()
}

/// Ensure a file exists in `zone`. No-op when present.
pub(crate) fn ensure_file(filestore: &FileStore, zone: &str, name: &str) -> Result<(), String> {
    match filestore.stat(zone, name) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => filestore
            .make_file(zone, name, FileMeta::default(), FileOpts::default())
            .map_err(|e| format!("make_file: {e}")),
        Err(e) => Err(format!("stat: {e}")),
    }
}

/// Write the entire contents of a file in `zone`. Creates the file if
/// missing, otherwise replaces all parts atomically (FileStore single-tx).
pub(crate) fn write_zone_file(
    filestore: &FileStore,
    zone: &str,
    name: &str,
    content: &[u8],
) -> Result<(), String> {
    use crate::backend::storage::StoreError;
    match filestore.write_file(zone, name, content) {
        Ok(()) => Ok(()),
        Err(StoreError::NotFound) => {
            filestore
                .make_file(zone, name, FileMeta::default(), FileOpts::default())
                .map_err(|e| format!("make_file: {e}"))?;
            filestore
                .write_file(zone, name, content)
                .map_err(|e| format!("write_file: {e}"))
        }
        Err(e) => Err(format!("write_file: {e}")),
    }
}
