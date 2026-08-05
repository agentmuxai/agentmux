// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Cross-process session-ownership leases — at most one live process
//! may drive turns for a given agent `instance_id` at a time.
//!
//! Sibling of [`super::store::Registry`] but deliberately a separate
//! file tree (`<registry_root>/leases/<instance_id>.{lease.json,lock}`),
//! not new fields on [`super::schema::NamedAgentRecordV1`]:
//! `Registry::upsert` is a blind read-modify-write (cross-process
//! safety comes only from atomic rename, no compare-and-swap — see
//! that module's doc comment), which is fine for the registry's own
//! last-writer-wins semantics but wrong for a lock, where "I thought
//! nobody held it" must be an atomic check.
//!
//! `claim`/`renew`/`release` each hold a real, blocking, per-`instance_id`
//! OS advisory lock (`flock` on Unix, `LockFileEx` on Windows — same
//! primitive family already used for single-instance enforcement in
//! `backend::base::WaveLock` and `agentmux-launcher::second_instance`,
//! just scoped per-lease instead of globally) for the short duration of
//! the call, not across calls. An earlier draft tried to get away with
//! pure atomic-rename/create-if-not-exists tricks and no real lock; it
//! had two real, independently-discovered races (a create-then-write
//! window a concurrent claimer could observe as "corrupt" and evict,
//! and a check-then-write TOCTOU in renew that could clobber a new
//! owner) — a real critical section is simpler AND correct, where the
//! lock-free version was neither.
//!
//! Root cause + design context:
//! `docs/retro/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md`,
//! `docs/analysis/ANALYSIS_MULTI_AGENT_SESSION_AND_WORKDIR_ISOLATION_2026-07-29.md`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::atomic::write_atomic;

/// How often a held lease must be renewed while a turn is in flight.
/// Piggybacks the existing per-turn health watchdog tick
/// (`blockcontroller/core.rs::spawn_health_watchdog`) — not a new timer.
pub const RENEW_INTERVAL_MS: u64 = 5_000;

/// 3x the renew interval — tolerates two missed ticks (disk hiccup, GC
/// pause) before a lease is considered abandoned and reclaimable.
pub const LEASE_TTL_MS: u64 = RENEW_INTERVAL_MS * 3;

#[derive(Debug, Error)]
pub enum LeaseError {
    #[error("session '{instance_id}' is already owned by another AgentMux process (boot {owner_boot_id}, renewed {age_ms}ms ago)")]
    HeldByOther {
        instance_id: String,
        owner_boot_id: String,
        age_ms: u64,
    },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Serialize, Deserialize)]
struct LeaseFile {
    instance_id: String,
    owner_boot_id: String,
    block_id: String,
    /// Informational only — NOT the lease key (see module doc comment
    /// and the analysis doc's Open Question this resolves: a
    /// session_id can be `None` on an agent's first-ever turn, which
    /// would leave that exact case unprotected if it were the key).
    session_id: Option<String>,
    pid: u32,
    acquired_at_ms: i64,
    renewed_at_ms: i64,
}

/// A held lease. Cheap to `Clone` (a couple of `String`s + a
/// `PathBuf`) — both the per-turn renew task and the eventual release
/// call need their own owned copy.
#[derive(Debug, Clone)]
pub struct Lease {
    instance_id: String,
    owner_boot_id: Arc<str>,
    path: PathBuf,
}

impl Lease {
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub struct LeaseStore {
    root: PathBuf,
    ttl_ms: u64,
}

impl LeaseStore {
    /// `<registry_root>/leases/`. Creates the directory if missing.
    pub fn open(registry_root: &Path) -> std::io::Result<Self> {
        let root = registry_root.join("leases");
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            ttl_ms: LEASE_TTL_MS,
        })
    }

    /// Test-only constructor with a small TTL so expiry tests don't
    /// need real multi-second sleeps.
    #[cfg(test)]
    fn open_with_ttl(registry_root: &Path, ttl_ms: u64) -> std::io::Result<Self> {
        let mut s = Self::open(registry_root)?;
        s.ttl_ms = ttl_ms;
        Ok(s)
    }

    fn content_path(&self, instance_id: &str) -> PathBuf {
        self.root.join(format!("{instance_id}.lease.json"))
    }

    fn lock_path(&self, instance_id: &str) -> PathBuf {
        self.root.join(format!("{instance_id}.lock"))
    }

    /// Read the lease file at `path`, if any. `Ok(None)` covers both
    /// "no file" and "unparseable file" — a corrupt lease is treated
    /// as having no trustworthy owner, same as a missing one; safe to
    /// overwrite once inside the critical section below.
    fn read(path: &Path) -> std::io::Result<Option<LeaseFile>> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(serde_json::from_slice(&bytes).ok())
    }

    /// Attempt to claim the lease for `instance_id`.
    ///
    /// - Already held by `boot_id` (this process) → idempotent
    ///   re-claim, timestamp refreshed.
    /// - Held by another owner, not yet expired → `Err(HeldByOther)`.
    /// - Free, corrupt, or expired → claimed.
    ///
    /// The whole read-decide-write sequence runs inside a per-
    /// `instance_id` exclusive lock (see module doc comment), so
    /// there's no window for a concurrent claimer to observe an
    /// inconsistent state.
    pub fn claim(
        &self,
        instance_id: &str,
        boot_id: &Arc<str>,
        block_id: &str,
        session_id_hint: Option<&str>,
    ) -> Result<Lease, LeaseError> {
        let _cs = CriticalSection::enter(&self.lock_path(instance_id))?;
        let path = self.content_path(instance_id);

        if let Some(existing) = Self::read(&path)? {
            if existing.owner_boot_id != boot_id.as_ref() {
                let age_ms = (now_ms() - existing.renewed_at_ms).max(0) as u64;
                if age_ms <= self.ttl_ms {
                    return Err(LeaseError::HeldByOther {
                        instance_id: instance_id.to_string(),
                        owner_boot_id: existing.owner_boot_id,
                        age_ms,
                    });
                }
                // Expired — falls through to the overwrite below.
            }
            // Same owner re-claiming (e.g. controller recreated on a
            // resync with no process restart), or an expired lease
            // from someone else — either way, safe to overwrite.
        }

        let bytes = lease_bytes(instance_id, boot_id, block_id, session_id_hint)?;
        write_atomic(&path, &bytes)?;
        Ok(Lease {
            instance_id: instance_id.to_string(),
            owner_boot_id: Arc::clone(boot_id),
            path,
        })
    }

    /// Extend a held lease's TTL. Runs inside the same per-`instance_id`
    /// critical section as `claim`, so the ownership check and the
    /// write are atomic with respect to any concurrent `claim`/`renew`/
    /// `release` for this instance — a caller that lost the lease to a
    /// TTL reclaim between its last renew and this one gets a real
    /// `HeldByOther`, never a silent clobber of the new owner's file.
    pub fn renew(&self, lease: &Lease) -> Result<(), LeaseError> {
        let _cs = CriticalSection::enter(&self.lock_path(&lease.instance_id))?;
        let Some(existing) = Self::read(&lease.path)? else {
            // Gone entirely — already reclaimed and later released, or
            // evicted. Nothing to renew.
            return Err(LeaseError::HeldByOther {
                instance_id: lease.instance_id.clone(),
                owner_boot_id: "<unknown — lease file missing>".to_string(),
                age_ms: 0,
            });
        };
        if existing.owner_boot_id != lease.owner_boot_id.as_ref() {
            let age_ms = (now_ms() - existing.renewed_at_ms).max(0) as u64;
            return Err(LeaseError::HeldByOther {
                instance_id: lease.instance_id.clone(),
                owner_boot_id: existing.owner_boot_id,
                age_ms,
            });
        }
        let renewed = LeaseFile {
            renewed_at_ms: now_ms(),
            ..existing
        };
        let mut bytes = serde_json::to_vec_pretty(&renewed).map_err(std::io::Error::from)?;
        bytes.push(b'\n');
        write_atomic(&lease.path, &bytes)?;
        Ok(())
    }

    /// Release a held lease. If it's no longer ours (already
    /// TTL-reclaimed by another process), this is a safe no-op —
    /// deleting would destroy the new owner's lease, not ours.
    /// Idempotent: releasing an already-missing lease is `Ok(())`.
    pub fn release(&self, lease: &Lease) -> Result<(), LeaseError> {
        let _cs = CriticalSection::enter(&self.lock_path(&lease.instance_id))?;
        let Some(existing) = Self::read(&lease.path)? else {
            return Ok(());
        };
        if existing.owner_boot_id != lease.owner_boot_id.as_ref() {
            return Ok(());
        }
        match std::fs::remove_file(&lease.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

fn lease_bytes(
    instance_id: &str,
    boot_id: &Arc<str>,
    block_id: &str,
    session_id_hint: Option<&str>,
) -> Result<Vec<u8>, std::io::Error> {
    let now = now_ms();
    let file = LeaseFile {
        instance_id: instance_id.to_string(),
        owner_boot_id: boot_id.to_string(),
        block_id: block_id.to_string(),
        session_id: session_id_hint.map(str::to_string),
        pid: std::process::id(),
        acquired_at_ms: now,
        renewed_at_ms: now,
    };
    let mut bytes = serde_json::to_vec_pretty(&file).map_err(std::io::Error::from)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// A held per-`instance_id` OS advisory lock. Blocking (waits rather
/// than failing immediately) — critical sections here are a handful of
/// small file ops, so worst-case wait time is negligible. Released
/// automatically on drop: closing the underlying handle releases both
/// `flock` (Unix) and `LockFileEx` (Windows) advisory locks.
struct CriticalSection {
    _file: std::fs::File,
}

impl CriticalSection {
    /// Deliberately never removes `lock_path` — one empty lock file
    /// persists per `instance_id` that has ever called `claim`, same
    /// bounded/accepted cost as `backend::base::WaveLock`'s single
    /// global lock file. This is NOT an oversight: deleting a flock/
    /// LockFileEx-locked file out from under a waiter is the classic
    /// "unlink race" footgun — a process blocked in `open()` (before
    /// it gets to actually lock) can end up holding a handle to a
    /// *different* underlying file than a fresh claimant that
    /// recreated the path after deletion, so both would believe they
    /// hold "the" lock while actually holding two distinct advisory
    /// locks on two distinct inodes. Leaving the lock file in place
    /// forever avoids that hazard entirely at the cost of a handful of
    /// permanently-empty files.
    fn enter(lock_path: &Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(lock_path)?;
        lock_exclusive(&file)?;
        Ok(Self { _file: file })
    }
}

#[cfg(unix)]
fn lock_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // Blocking (no LOCK_NB) — we want to wait for the holder, not fail.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn lock_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{LockFileEx, LOCKFILE_EXCLUSIVE_LOCK};
    use windows_sys::Win32::System::IO::OVERLAPPED;

    let handle = file.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
    // Zeroed OVERLAPPED with offset 0 + max length locks the whole
    // file. LockFileEx requires an OVERLAPPED struct even for a
    // synchronous, blocking call (offset fields only matter for
    // partial-file locks, which we don't use).
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    // No LOCKFILE_FAIL_IMMEDIATELY — blocks until the lock is free.
    let ok = unsafe {
        LockFileEx(handle, LOCKFILE_EXCLUSIVE_LOCK, 0, u32::MAX, u32::MAX, &mut overlapped)
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(ttl_ms: u64) -> (tempfile::TempDir, LeaseStore) {
        let tmp = tempfile::tempdir().unwrap();
        let s = LeaseStore::open_with_ttl(tmp.path(), ttl_ms).unwrap();
        (tmp, s)
    }

    fn boot(id: &str) -> Arc<str> {
        Arc::from(id)
    }

    #[test]
    fn claim_succeeds_when_no_existing_lease() {
        let (_tmp, s) = store(60_000);
        let lease = s.claim("agent-1", &boot("boot-a"), "block-1", None).unwrap();
        assert_eq!(lease.instance_id(), "agent-1");
    }

    #[test]
    fn claim_fails_when_held_by_other_and_fresh() {
        let (_tmp, s) = store(60_000);
        s.claim("agent-1", &boot("boot-a"), "block-1", None).unwrap();
        let err = s.claim("agent-1", &boot("boot-b"), "block-2", None).unwrap_err();
        match err {
            LeaseError::HeldByOther { owner_boot_id, .. } => assert_eq!(owner_boot_id, "boot-a"),
            other => panic!("expected HeldByOther, got {other:?}"),
        }
    }

    #[test]
    fn claim_is_idempotent_for_same_owner() {
        let (_tmp, s) = store(60_000);
        s.claim("agent-1", &boot("boot-a"), "block-1", None).unwrap();
        let second = s.claim("agent-1", &boot("boot-a"), "block-1", None);
        assert!(second.is_ok());
    }

    #[test]
    fn claim_reclaims_after_ttl_expiry() {
        // Wide margin (20x) — under a full parallel `cargo test` run,
        // thread::sleep can overshoot by tens of ms under scheduler
        // contention; a tight TTL/sleep ratio here was observed flaky.
        let (_tmp, s) = store(50);
        s.claim("agent-1", &boot("boot-a"), "block-1", None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1_000));
        let reclaimed = s.claim("agent-1", &boot("boot-b"), "block-2", None);
        assert!(reclaimed.is_ok(), "expected expired lease to be reclaimable, got {reclaimed:?}");
    }

    #[test]
    fn corrupt_lease_file_is_treated_as_stale_and_evicted() {
        let (tmp, s) = store(60_000);
        let path = tmp.path().join("leases").join("agent-1.lease.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not json").unwrap();
        let claimed = s.claim("agent-1", &boot("boot-a"), "block-1", None);
        assert!(claimed.is_ok(), "expected corrupt lease to be evicted, got {claimed:?}");
    }

    #[test]
    fn renew_extends_ttl_and_blocks_reclaim_by_other() {
        // Large TTL relative to the sleeps (100x+ margin) — this test
        // asserts the lease is still considered FRESH, so it needs the
        // opposite safety direction from the expiry tests: elapsed
        // time must stay comfortably under the TTL even with generous
        // scheduler jitter on a busy `cargo test` run.
        let (_tmp, s) = store(5_000);
        let lease = s.claim("agent-1", &boot("boot-a"), "block-1", None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        s.renew(&lease).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        // ~40ms elapsed since claim, ~20ms since the renew — still fresh.
        let err = s.claim("agent-1", &boot("boot-b"), "block-2", None).unwrap_err();
        assert!(matches!(err, LeaseError::HeldByOther { .. }));
    }

    #[test]
    fn renew_fails_when_already_reclaimed_by_other() {
        let (tmp, s) = store(50);
        let lease = s.claim("agent-1", &boot("boot-a"), "block-1", None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1_000));
        s.claim("agent-1", &boot("boot-b"), "block-2", None).unwrap();

        let err = s.renew(&lease).unwrap_err();
        assert!(matches!(err, LeaseError::HeldByOther { .. }));

        // Must NOT have clobbered boot-b's lease.
        let path = tmp.path().join("leases").join("agent-1.lease.json");
        let bytes = std::fs::read(&path).unwrap();
        let on_disk: LeaseFile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(on_disk.owner_boot_id, "boot-b");
    }

    #[test]
    fn release_removes_the_lease_file() {
        let (tmp, s) = store(60_000);
        let lease = s.claim("agent-1", &boot("boot-a"), "block-1", None).unwrap();
        s.release(&lease).unwrap();
        let path = tmp.path().join("leases").join("agent-1.lease.json");
        assert!(!path.exists());
    }

    #[test]
    fn release_is_a_noop_when_already_reclaimed_by_other() {
        let (tmp, s) = store(50);
        let lease = s.claim("agent-1", &boot("boot-a"), "block-1", None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1_000));
        s.claim("agent-1", &boot("boot-b"), "block-2", None).unwrap();

        s.release(&lease).unwrap();

        let path = tmp.path().join("leases").join("agent-1.lease.json");
        assert!(path.exists(), "release must not delete another owner's lease");
        let bytes = std::fs::read(&path).unwrap();
        let on_disk: LeaseFile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(on_disk.owner_boot_id, "boot-b");
    }

    #[test]
    fn release_is_idempotent_on_missing_file() {
        let (_tmp, s) = store(60_000);
        let lease = s.claim("agent-1", &boot("boot-a"), "block-1", None).unwrap();
        s.release(&lease).unwrap();
        assert!(s.release(&lease).is_ok());
    }

    #[test]
    fn concurrent_claim_race_has_exactly_one_winner() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let n = 8;
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let root = root.clone();
                std::thread::spawn(move || {
                    let s = LeaseStore::open(&root).unwrap();
                    s.claim("agent-1", &boot(&format!("boot-{i}")), "block", None)
                        .is_ok()
                })
            })
            .collect();
        let wins: usize = handles.into_iter().map(|h| h.join().unwrap()).filter(|ok| *ok).count();
        assert_eq!(wins, 1, "exactly one claimant should win the race");
    }
}
