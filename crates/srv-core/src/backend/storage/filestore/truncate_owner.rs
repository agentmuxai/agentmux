// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which srv on the machine truncates a shared store's WAL. Every srv opens the
//! global transcript store, and one TRUNCATE checkpoint every half hour is enough:
//! the first srv to take an advisory lock on a file beside the database keeps it
//! for as long as it runs, and the others leave the job to it. When it exits the
//! operating system drops the lock, and another srv takes it at its next look.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;

use super::core::FileStore;

pub struct TruncateOwner {
    lock_path: PathBuf,
    held: Option<File>,
}

impl TruncateOwner {
    /// `None` for an in-memory store (nothing shared to truncate).
    pub fn for_store(store: &FileStore) -> Option<Self> {
        let db = store.path()?;
        let mut name = db.as_os_str().to_os_string();
        name.push(".owner");
        Some(Self { lock_path: PathBuf::from(name), held: None })
    }

    /// Whether this srv is the owner, taking the lock if nobody holds it.
    pub fn is_owner(&mut self) -> bool {
        if self.held.is_some() {
            return true;
        }
        let Ok(file) = OpenOptions::new().create(true).truncate(false).write(true).open(&self.lock_path) else {
            return false;
        };
        match file.try_lock() {
            Ok(()) => {
                self.held = Some(file);
                true
            }
            Err(TryLockError::WouldBlock) => false,
            Err(TryLockError::Error(e)) => {
                tracing::debug!(error = %e, path = %self.lock_path.display(), "filestore: truncate-owner lock failed");
                false
            }
        }
    }
}
