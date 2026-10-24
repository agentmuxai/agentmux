// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A cross-process lock per shared definition record
//! (`docs/specs/SPEC_FORK_SLUG_MIGRATION_2026_10_22.md` P0.2).
//!
//! Every channel and instance on the host writes these records, each with its
//! own `DefinitionStore`. An update reads the record, keeps its slug and
//! former slugs (`keep_slug_and_former_slugs`), and writes it back. Without a
//! lock spanning all three, another writer can land in between, and the
//! write-back reverts its slug or drops a former slug it just recorded. The
//! lock is an OS file lock (`LockFileEx` / `flock`, through `std`) on
//! `<root>/locks/<id>.lock`, which binds one open handle, so it serializes
//! writers in this process as well as in others. Lock files are never removed:
//! deleting one while another writer waits on it would hand two writers two
//! different locks.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::time::{Duration, Instant};

use super::def_store::DefStoreError;

/// How long a writer waits before giving up with `DefStoreError::Locked`.
/// A write holds the lock for one small file's read and atomic replace.
pub(super) const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

const RETRY: Duration = Duration::from_millis(10);

/// Record `id`'s lock, held until the returned file is dropped.
pub(super) fn lock_record(root: &Path, id: &str, timeout: Duration) -> Result<File, DefStoreError> {
    let dir = root.join("locks");
    std::fs::create_dir_all(&dir)?;
    #[allow(clippy::suspicious_open_options)]
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(format!("{id}.lock")))?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) if started.elapsed() < timeout => std::thread::sleep(RETRY),
            Err(TryLockError::WouldBlock) => {
                return Err(DefStoreError::Locked {
                    id: id.to_string(),
                    waited_ms: started.elapsed().as_millis() as u64,
                })
            }
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
}

/// The lock name for slug allocation (a uid is a UUID, so it can't collide).
pub(super) const SLUG_ALLOCATION: &str = "_slug-allocation";
/// The lock name for seat changes: who reports to whom is one graph.
pub(super) const SEAT_GRAPH: &str = "_seat-graph";
