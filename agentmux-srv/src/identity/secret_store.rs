// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! OS-native secret storage for Armory API keys.
//!
//! Backs `SecretRef::Keychain { service, account }`. The plaintext key
//! lives only in the OS keychain (macOS Keychain / Windows Credential
//! Manager / Linux Secret Service via the `keyring` crate); the DB holds
//! just the pointer plus non-secret metadata + a masked tail. Reads return
//! a `Zeroizing<String>` so the plaintext is wiped from memory on drop.
//!
//! See specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §7 (best practices) and §12.2.
//!
//! NOTE: an encrypted-file fallback for headless Linux without a Secret
//! Service agent is a documented follow-up (spec §12.2); the desktop app
//! ships with a keychain on all three platforms, so keyring is the path
//! here. Failures surface as a typed error rather than a silent downgrade.
//!
//! **Bounded by [`TIMEOUT`]**: every operation below can require
//! interactive OS consent ("App wants to access your confidential
//! information...") the first time a given code signature touches a given
//! entry — and that consent call has no cancellation mechanism, so an
//! unanswered prompt (headless process, dialog on another Space, no
//! attached display session) blocks the underlying platform call
//! indefinitely. Confirmed live — see
//! `docs/retro/retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md` §5.
//! Every public function here runs the real platform call on a detached
//! thread and gives up waiting after `TIMEOUT`, so a stuck prompt bounds
//! how long the CALLER waits (and any lock it's holding) instead of hanging
//! it forever. The detached thread itself is NOT killed — it keeps running
//! until the OS resolves it (answered or not) — this is a wait-bound, not a
//! true cancellation; there is no way to cancel the underlying platform
//! call itself.

use std::sync::mpsc;
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

/// Store (or overwrite) the secret for `account_id` in the OS keychain.
pub fn put(account_id: &str, secret: &str) -> Result<(), String> {
    let account_id = account_id.to_string();
    // reagent P1: wrap the plaintext BEFORE moving it into the detached
    // thread's closure, not after — moving a plain `String` there left an
    // extra, unscrubbed heap copy of the secret alive for up to the full
    // timeout on a stuck call (a real widening of the plaintext-exposure
    // window versus the pre-timeout code, which passed the caller's `&str`
    // straight through with no extra copy at all). `Zeroizing` here makes
    // this copy scrub itself on drop, same guarantee `get`/`get_optional`
    // already give their own copies.
    let secret = Zeroizing::new(secret.to_string());
    match run_with_timeout(TIMEOUT, move || put_now(&account_id, &secret)) {
        Ok(result) => result,
        Err(outcome) => Err(run_outcome_message("write", outcome)),
    }
}

fn put_now(account_id: &str, secret: &str) -> Result<(), String> {
    entry(account_id)?
        .set_password(secret)
        .map_err(|e| format!("keychain write failed: {e}"))
}

/// Read the secret for `account_id`. Returned wrapped in `Zeroizing` so it
/// is wiped on drop. Resolved at agent spawn time when injecting env vars.
pub fn get(account_id: &str) -> Result<Zeroizing<String>, String> {
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
pub fn delete(account_id: &str) -> Result<(), String> {
    let account_id = account_id.to_string();
    match run_with_timeout(TIMEOUT, move || delete_now(&account_id)) {
        Ok(result) => result,
        Err(outcome) => Err(run_outcome_message("delete", outcome)),
    }
}

fn delete_now(account_id: &str) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
