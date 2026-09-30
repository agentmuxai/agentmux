// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! srv's secret storage: Armory API keys, OAuth accounts, MuxBus
//! credentials, browser-pane logins.
//!
//! Backs `SecretRef::Keychain { service, account }`, which names an entry in
//! *this store*, wherever it keeps secrets; the DB holds just the pointer plus
//! non-secret metadata + a masked tail. Reads return a `Zeroizing<String>` so
//! the plaintext is wiped from memory on drop.
//!
//! **Two backends**, chosen once at startup:
//! - **The OS keychain** (macOS Keychain / Windows Credential Manager / Linux
//!   Secret Service via the `keyring` crate). The default, and what the
//!   desktop app always uses.
//! - **A file store** ([`use_file_store`]): one `0600` file per entry in a
//!   `0700` directory, for machines with no keychain — a container, a
//!   server, CI. Headless srv uses it unless told otherwise
//!   (docs/specs/SPEC_SRV_HEADLESS_MODE_2026_09_26.md §3.6). Secrets are
//!   protected by file permissions (and the volume's own encryption), not
//!   encrypted by srv: a key kept beside them would add nothing.
//!
//! There is no silent downgrade from one to the other: failures surface as
//! errors. See docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §7 and §12.2.
//!
//! The rest of this comment is about the keychain backend.
//!
//! **Reads are bounded by [`TIMEOUT`]; writes are not — see below.** Every
//! operation here can require interactive OS consent ("App wants to access
//! your confidential information...") the first time a given code
//! signature touches a given entry, and that consent call has no
//! cancellation mechanism: an unanswered prompt (headless process, dialog
//! on another Space, no attached display session) blocks the underlying
//! platform call indefinitely. Confirmed live — see
//! `docs/retro/retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md` §5.
//!
//! `get`/`get_optional` run the real platform call on a detached thread and
//! give up waiting after `TIMEOUT`, so a stuck prompt bounds how long the
//! CALLER waits (and any lock it's holding) instead of hanging it forever.
//! This is safe specifically because a read has no side effect: if the
//! timeout fires, the detached thread's eventual (possibly much later)
//! result is just discarded — nothing acts on a stale read.
//!
//! `put`/`delete` deliberately do NOT use this timeout (reagent + Codex,
//! PR #2679 round 2): the same "detached thread keeps running after the
//! caller gives up" behavior is unsafe for a MUTATION. A caller that treats
//! a timed-out write as failure and proceeds — e.g. skipping a dependent DB
//! write, or a rollback in `muxbus.rs` writing a different value to the
//! same entry — can have the orphaned original write land afterward and
//! silently clobber whatever the caller did next, with no ordering
//! guarantee between them. There is no cancellation-equivalent compensation
//! available (the platform call truly cannot be cancelled), so until one
//! exists, an unbounded wait — the pre-existing, safe-if-slow behavior — is
//! the correct tradeoff for anything that mutates state. A stuck consent
//! prompt on a write still blocks its caller indefinitely; only reads are
//! protected today.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use keyring::Entry;
use zeroize::Zeroizing;

/// Keychain service string — constant across all AgentMux secrets.
pub const SERVICE: &str = "agentmux";

/// How long a caller waits for a keychain operation before giving up. Long
/// enough for a real user to notice and answer a genuine first-time consent
/// prompt if they're looking at their screen; short enough that an
/// unanswered/unanswerable one doesn't wedge whatever the caller is holding
/// (e.g. `muxbus_save_lock`) indefinitely. See this module's doc comment.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Why `run_with_timeout` gave up waiting — reagent P2: collapsing these
/// into one outcome made a closure PANIC (sender dropped without sending,
/// `mpsc::RecvTimeoutError::Disconnected`) misreport as "timed out after
/// 15s" — actively misleading, since the operation actually failed
/// immediately, not after waiting the full deadline.
#[derive(Debug, PartialEq)]
enum RunOutcome {
    /// The deadline elapsed with no answer yet — the likely "stuck consent
    /// prompt" case this module exists to bound.
    TimedOut,
    /// The worker thread's sender was dropped without sending — it panicked
    /// before calling `f` to completion.
    WorkerPanicked,
}

/// Run `f` on a detached thread and wait up to `timeout` for it. The
/// `RunOutcome` distinction is separate from any error `f` itself can
/// return — callers fold it into their own error type at the call site
/// (see `put`/`get`/`get_optional`/`delete` below), not here, so this stays
/// reusable for a future caller with a different error shape.
fn run_with_timeout<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, RunOutcome> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // The receiver may already be gone (we timed out and moved on) —
        // a failed send here just means nobody's listening anymore, not a
        // bug; the value is dropped.
        let _ = tx.send(f());
    });
    rx.recv_timeout(timeout).map_err(|e| match e {
        mpsc::RecvTimeoutError::Timeout => RunOutcome::TimedOut,
        mpsc::RecvTimeoutError::Disconnected => RunOutcome::WorkerPanicked,
    })
}

fn entry(account_id: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, &account_key(account_id))
        .map_err(|e| format!("keychain entry init failed: {e}"))
}

/// Build the keychain account string for an identity-account id.
pub fn account_key(account_id: &str) -> String {
    format!("acct:{account_id}")
}

/// Set when [`use_file_store`] chose the file store; unset means the keychain.
static FILE_STORE: OnceLock<PathBuf> = OnceLock::new();

/// Keep every secret in files under `dir` instead of the OS keychain, for
/// the rest of this process. Called once at startup, before anything reads
/// or writes a secret. Creates `dir` (mode `0700`) if needed.
pub fn use_file_store(dir: PathBuf) -> Result<(), String> {
    file::prepare_dir(&dir)?;
    FILE_STORE
        .set(dir)
        .map_err(|_| "the secret store was already chosen".to_string())
}

/// Which backend this process uses, for logs: `"keychain"` or `"file"`.
pub fn backend_name() -> &'static str {
    if FILE_STORE.get().is_some() {
        "file"
    } else {
        "keychain"
    }
}

/// Store (or overwrite) the secret for `account_id` in the OS keychain.
///
/// Deliberately unbounded — see this module's doc comment for why a
/// timeout is unsafe for a mutation specifically (a timed-out-but-later-
/// completing write can land after the caller has already acted on the
/// assumption it failed).
pub fn put(account_id: &str, secret: &str) -> Result<(), String> {
    if let Some(dir) = FILE_STORE.get() {
        return file::put(dir, &account_key(account_id), secret);
    }
    entry(account_id)?
        .set_password(secret)
        .map_err(|e| format!("keychain write failed: {e}"))
}

/// Read the secret for `account_id`. Returned wrapped in `Zeroizing` so it
/// is wiped on drop. Resolved at agent spawn time when injecting env vars.
pub fn get(account_id: &str) -> Result<Zeroizing<String>, String> {
    if let Some(dir) = FILE_STORE.get() {
        return file::get_optional(dir, &account_key(account_id))?
            .ok_or_else(|| "secret store read failed: no entry for this account".to_string());
    }
    let account_id = account_id.to_string();
    match run_with_timeout(TIMEOUT, move || get_now(&account_id)) {
        Ok(result) => result,
        Err(outcome) => Err(run_outcome_message("read", outcome)),
    }
}

fn get_now(account_id: &str) -> Result<Zeroizing<String>, String> {
    let pw = entry(account_id)?
        .get_password()
        .map_err(|e| format!("keychain read failed: {e}"))?;
    Ok(Zeroizing::new(pw))
}

/// Read the secret for `account_id`, distinguishing "no entry stored yet"
/// (`Ok(None)`) from a real storage failure (`Err`) — a locked keychain, no
/// Secret Service daemon running, permission denied, etc. `get` collapses
/// both into the same `Err` variant, which is correct for its own callers
/// (a spawn-time credential resolve should fail either way), but is the
/// wrong shape for a caller that needs to tell "genuinely never logged in"
/// apart from "storage is transiently broken" — treating a transient
/// failure as "no credential" can silently present as a full logout. Use
/// this variant when that distinction matters.
pub fn get_optional(account_id: &str) -> Result<Option<Zeroizing<String>>, String> {
    if let Some(dir) = FILE_STORE.get() {
        return file::get_optional(dir, &account_key(account_id));
    }
    let account_id = account_id.to_string();
    match run_with_timeout(TIMEOUT, move || get_optional_now(&account_id)) {
        Ok(result) => result,
        Err(outcome) => Err(run_outcome_message("read", outcome)),
    }
}

fn get_optional_now(account_id: &str) -> Result<Option<Zeroizing<String>>, String> {
    match entry(account_id)?.get_password() {
        Ok(pw) => Ok(Some(Zeroizing::new(pw))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("keychain read failed: {e}")),
    }
}

/// Delete the secret for `account_id`. A missing entry is treated as
/// success (idempotent delete).
///
/// Deliberately unbounded — see this module's doc comment for why a
/// timeout is unsafe for a mutation specifically.
pub fn delete(account_id: &str) -> Result<(), String> {
    if let Some(dir) = FILE_STORE.get() {
        return file::delete(dir, &account_key(account_id));
    }
    match entry(account_id)?.delete_password() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keychain delete failed: {e}")),
    }
}

fn run_outcome_message(op: &str, outcome: RunOutcome) -> String {
    match outcome {
        RunOutcome::TimedOut => format!(
            "keychain {op} timed out after {TIMEOUT:?} — likely an unanswered OS access-consent \
             prompt (see docs/retro/retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md §5)"
        ),
        RunOutcome::WorkerPanicked => {
            format!("keychain {op} failed: the background thread performing it panicked")
        }
    }
}

/// The file store. Each entry is one file named by the SHA-256 of its
/// account key, so no key text (ids, hostnames) reaches a file name and no
/// key can escape the directory. No timeouts: file operations don't wait
/// on a user.
mod file {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    pub(super) fn path_for(dir: &Path, key: &str) -> PathBuf {
        let digest = Sha256::digest(key.as_bytes());
        dir.join(digest.iter().map(|b| format!("{b:02x}")).collect::<String>())
    }

    /// Create `dir` if needed and make it owner-only (`0700`), also when it
    /// already existed with wider permissions.
    pub(super) fn prepare_dir(dir: &Path) -> Result<(), String> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(dir)
            .map_err(|e| format!("secret store: cannot create {}: {e}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("secret store: cannot restrict {}: {e}", dir.display()))?;
        }
        Ok(())
    }

    /// Write via a fresh `0600` temp file and a rename, so a reader never
    /// sees a partial secret and a crash never leaves one behind.
    pub(super) fn put(dir: &Path, key: &str, secret: &str) -> Result<(), String> {
        let path = path_for(dir, key);
        let tmp = dir.join(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let written = opts
            .open(&tmp)
            .and_then(|mut f| f.write_all(secret.as_bytes()).and_then(|()| f.sync_all()))
            .and_then(|()| std::fs::rename(&tmp, &path));
        written.map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("secret store write failed: {e}")
        })
    }

    pub(super) fn get_optional(dir: &Path, key: &str) -> Result<Option<Zeroizing<String>>, String> {
        match std::fs::read(path_for(dir, key)) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(|s| Some(Zeroizing::new(s)))
                .map_err(|_| "secret store read failed: the entry is not valid UTF-8".to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("secret store read failed: {e}")),
        }
    }

    pub(super) fn delete(dir: &Path, key: &str) -> Result<(), String> {
        match std::fs::remove_file(path_for(dir, key)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("secret store delete failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_round_trips_overwrites_and_deletes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("secrets");
        file::prepare_dir(&dir).unwrap();
        let key = account_key("abc");
        assert!(file::get_optional(&dir, &key).unwrap().is_none());
        file::put(&dir, &key, "first").unwrap();
        assert_eq!(file::get_optional(&dir, &key).unwrap().as_deref().map(String::as_str), Some("first"));
        file::put(&dir, &key, "second").unwrap();
        assert_eq!(file::get_optional(&dir, &key).unwrap().as_deref().map(String::as_str), Some("second"));
        file::delete(&dir, &key).unwrap();
        assert!(file::get_optional(&dir, &key).unwrap().is_none());
        // Deleting what isn't there is fine, as with the keychain.
        file::delete(&dir, &key).unwrap();
        // Only the entries themselves are left: no temp files.
        file::put(&dir, &key, "x").unwrap();
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }

    #[test]
    fn file_store_entries_are_separate_and_named_by_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        // Keys with separators and traversal text stay inside the directory.
        let (a, b) = ("muxbus:global", "browser-auth:id:../../etc/passwd");
        file::put(&dir, a, "A").unwrap();
        file::put(&dir, b, "B").unwrap();
        assert_eq!(file::get_optional(&dir, a).unwrap().as_deref().map(String::as_str), Some("A"));
        assert_eq!(file::get_optional(&dir, b).unwrap().as_deref().map(String::as_str), Some("B"));
        for key in [a, b] {
            let p = file::path_for(&dir, key);
            assert_eq!(p.parent(), Some(dir.as_path()));
            let name = p.file_name().unwrap().to_str().unwrap().to_string();
            assert_eq!(name.len(), 64);
            assert!(name.bytes().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("secrets");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        // An existing, wider directory is tightened.
        file::prepare_dir(&dir).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        file::put(&dir, "k", "v").unwrap();
        assert_eq!(mode(&file::path_for(&dir, "k")), 0o600);
    }

    #[test]
    fn account_key_is_namespaced() {
        assert_eq!(account_key("abc123"), "acct:abc123");
    }

    #[test]
    fn run_with_timeout_returns_the_value_when_the_work_finishes_in_time() {
        let result = run_with_timeout(Duration::from_secs(5), || 42);
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn run_with_timeout_gives_up_when_the_work_outlives_the_deadline() {
        let result: Result<(), RunOutcome> = run_with_timeout(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_secs(5));
        });
        assert_eq!(result, Err(RunOutcome::TimedOut));
    }

    #[test]
    fn run_with_timeout_reports_a_panic_distinctly_from_a_timeout() {
        // reagent P2: a panicking closure must not be misreported as
        // "timed out" — it failed immediately, not after waiting the full
        // deadline. Give it a generous deadline so a slow test runner can't
        // turn this into a race against the (much shorter) real timeout.
        let result: Result<(), RunOutcome> = run_with_timeout(Duration::from_secs(5), || {
            panic!("simulated worker panic");
        });
        assert_eq!(result, Err(RunOutcome::WorkerPanicked));
    }

    #[test]
    fn run_outcome_message_distinguishes_timeout_from_panic() {
        assert!(run_outcome_message("read", RunOutcome::TimedOut).contains("timed out"));
        assert!(run_outcome_message("read", RunOutcome::WorkerPanicked).contains("panicked"));
    }
}
