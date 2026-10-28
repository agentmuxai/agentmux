// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One refresh at a time per Claude config dir (#6081).
//!
//! The Claude CLI refreshes its own OAuth token, and the refresh token rotates: a refresh spends
//! the old one. Two CLIs that start on the same `CLAUDE_CONFIG_DIR` while its access token has
//! expired both refresh; the second presents the token the first just spent, is refused, and
//! can leave the dir signed out. A boot does exactly that: every pane that shares an identity
//! resumes within milliseconds, and an ambient side call (Haiku) joins them.
//!
//! So while a dir's token is expired, one process goes first and the rest wait:
//!   - the first spawn finds no entry and an expired token, and takes the dir (`Entry::owner` is
//!     its block). It runs at once; its CLI does the refresh;
//!   - a later spawn on that dir waits until the token is fresh again, the owner's first turn has
//!     ended or its process is gone, or [`WARM_MAX`] has passed. Those that waited start
//!     [`STAGGER`] apart, so a dozen panes do not all hit the API in the same millisecond;
//!   - an ambient call waits until the owner's turn has ended, whether or not the token is fresh
//!     by then, and takes the dir itself when the token is expired and nobody has it.
//!
//! This never refreshes a token itself and never blocks for good: a dead refresh token, or an
//! owner that never reports back, costs one [`WARM_MAX`] and then the next caller owns the dir.
//! The wait comes before the account is resolved (`inject_after_warmup`), so the status the
//! spawn gate reads is the one after the refresh, not before it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::time::{sleep_until, timeout, Instant};

use crate::backend::mps::Broker;
use crate::backend::storage::store::Store;
use crate::identity::resolver::{OAuthProbeStatus, SpawnGateError};

/// The longest a dir stays taken by a process that has not reported its first turn over.
pub const WARM_MAX: Duration = Duration::from_secs(90);
/// The longest it stays taken by a spawn whose process has not started: the time the gate and the
/// env take. A spawn that fails after the dir was taken (a gate refusal, a deleted agent) holds it
/// no longer than this.
pub const GATE_GRACE: Duration = Duration::from_secs(30);
/// How often a waiter looks at the token file, which nothing signals when it changes.
const POLL: Duration = Duration::from_millis(500);
/// The gap between spawns that waited on the same dir.
pub const STAGGER: Duration = Duration::from_millis(250);
/// A token this close to expiry counts as expired: the CLI refreshes inside its own margin.
const STALE_MARGIN_MS: i64 = 5 * 60 * 1000;

struct Entry {
    owner: String,
    deadline: Instant,
}

enum Step {
    Go,
    Wait(Instant),
}

#[derive(Default)]
struct Inner {
    taken: HashMap<PathBuf, Entry>,
    next_release: HashMap<PathBuf, Instant>,
}

#[derive(Default)]
struct Warmup {
    inner: Mutex<Inner>,
    wake: tokio::sync::Notify,
}

impl Warmup {
    /// Decide for `owner` on `dir`. `through_turn` is an ambient call's rule: it waits for the
    /// owner's turn to end even once the token is fresh.
    fn step(&self, dir: &Path, owner: &str, stale: bool, through_turn: bool, now: Instant) -> Step {
        let mut g = self.inner.lock().unwrap();
        if g.taken.get(dir).is_some_and(|e| e.deadline <= now) {
            if let Some(e) = g.taken.remove(dir) {
                tracing::warn!(
                    target: "identity",
                    dir = %dir.display(),
                    owner = %e.owner,
                    "token warm-up: owner never reported its first turn over — dir released"
                );
            }
        }
        match g.taken.get(dir) {
            Some(e) if e.owner == owner => Step::Go,
            Some(e) if stale || through_turn => Step::Wait(e.deadline),
            Some(_) => Step::Go,
            None => {
                if stale {
                    tracing::info!(
                        target: "identity",
                        dir = %dir.display(),
                        owner,
                        "token warm-up: access token expired — {owner} goes first, others on this dir wait"
                    );
                    let hold = if through_turn { WARM_MAX } else { GATE_GRACE };
                    g.taken.insert(dir.to_path_buf(), Entry { owner: owner.to_string(), deadline: now + hold });
                }
                Step::Go
            }
        }
    }

    /// The slot a caller that waited starts at: [`STAGGER`] after the last one on this dir.
    fn reserve_release(&self, dir: &Path, now: Instant) -> Instant {
        let mut g = self.inner.lock().unwrap();
        let at = g.next_release.get(dir).copied().filter(|t| *t > now).unwrap_or(now);
        g.next_release.insert(dir.to_path_buf(), at + STAGGER);
        at
    }

    async fn enter(&self, dir: &Path, owner: &str, through_turn: bool, is_stale: impl Fn() -> bool) {
        // A caller dropped while it waited out its stagger must not leave the dir it took held.
        let mut undo = ReleaseOnDrop { warmup: self, owner, armed: true };
        let mut waited = false;
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let now = Instant::now();
            match self.step(dir, owner, is_stale(), through_turn, now) {
                Step::Go => break,
                Step::Wait(deadline) => {
                    waited = true;
                    let _ = timeout(POLL.min(deadline.saturating_duration_since(now)), notified).await;
                }
            }
        }
        if waited {
            sleep_until(self.reserve_release(dir, Instant::now())).await;
        }
        undo.armed = false;
    }

    /// `owner`'s process is up: it now holds its dir for the first turn, up to [`WARM_MAX`].
    fn started(&self, owner: &str) {
        let mut g = self.inner.lock().unwrap();
        let until = Instant::now() + WARM_MAX;
        for e in g.taken.values_mut().filter(|e| e.owner == owner) {
            e.deadline = until;
        }
    }

    /// Free every dir `owner` holds. True if there was one.
    fn release(&self, owner: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        let before = g.taken.len();
        g.taken.retain(|_, e| e.owner != owner);
        let released = g.taken.len() != before;
        drop(g);
        if released {
            self.wake.notify_waiters();
        }
        released
    }
}

struct ReleaseOnDrop<'a> {
    warmup: &'a Warmup,
    owner: &'a str,
    armed: bool,
}

impl Drop for ReleaseOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.warmup.release(self.owner);
        }
    }
}

/// The map key for a config dir: the same dir spelled two ways (a trailing separator, the other
/// slash, case on Windows) is one dir. An ambient call names its dir from the block's `cmd:env`, a
/// spawn from the bound account; they must meet.
fn key_of(dir: &Path) -> PathBuf {
    let s = dir.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/');
    PathBuf::from(if cfg!(windows) { s.to_lowercase() } else { s.to_string() })
}

fn global() -> &'static Warmup {
    static GLOBAL: OnceLock<Warmup> = OnceLock::new();
    GLOBAL.get_or_init(Warmup::default)
}

/// Whether the Claude token in `dir` has expired, or will within the CLI's own margin, and holds
/// a refresh token to renew it with. Anything else (fresh, no file, no refresh token, a keychain
/// platform) is not ours to wait on.
fn token_is_stale(dir: &Path) -> bool {
    let now_ms = agentmux_common::time::now_ms();
    crate::identity::resolver::probe_oauth_status("claude", &dir.to_string_lossy(), now_ms + STALE_MARGIN_MS)
        == Some(OAuthProbeStatus::Expired)
}

/// A spawn on `block_id`'s bound Claude dir is over, or its first turn is: whoever waited on it
/// may go. Cheap and safe to call on every turn end and every process exit.
pub fn release(block_id: &str) {
    global().release(block_id);
}

/// The CLI process for `block_id` is running: its first turn may take [`WARM_MAX`] to end.
pub fn started(block_id: &str) {
    global().started(block_id);
}

/// `inject_identity_env_async`, after waiting for this block's dir: the single-flight comes
/// first, so the account the gate resolves and the status it probes are the ones after the
/// refresh. A block with no bound Claude dir goes straight through.
pub async fn inject_after_warmup(
    mstore: Arc<Store>,
    id_store: Arc<Store>,
    identity_store: Arc<Store>,
    broker: Option<Arc<Broker>>,
    block_id: String,
    env_vars: HashMap<String, String>,
) -> Result<HashMap<String, String>, SpawnGateError> {
    let dir = {
        let (m, i, s, b) = (mstore.clone(), id_store.clone(), identity_store.clone(), block_id.clone());
        tokio::task::spawn_blocking(move || crate::identity::resolver::resolve_bound_oauth_config_dir(&m, &i, &s, &b))
            .await
            .ok()
            .flatten()
    };
    if let Some(dir) = dir {
        let probe = dir.clone();
        global().enter(&key_of(&dir), &block_id, false, move || token_is_stale(&probe)).await;
    }
    let result = crate::identity::resolver::inject_identity_env_async(
        mstore, id_store, identity_store, broker, block_id.clone(), env_vars,
    )
    .await;
    if result.is_err() {
        // The gate refused: no process will start, so the dir this spawn took is nobody's.
        release(&block_id);
    }
    result
}

/// Holds a dir for an ambient call while its token is expired; frees it when dropped.
pub struct AmbientGuard {
    owner: String,
}

impl Drop for AmbientGuard {
    fn drop(&mut self) {
        release(&self.owner);
    }
}

/// Wait before an ambient CLI runs on `config_dir` (its `CLAUDE_CONFIG_DIR`), until the first
/// spawn on that dir has had its turn. `None`: the call has no dir of its own to protect.
pub async fn ambient_gate(config_dir: Option<&str>) -> AmbientGuard {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let owner = format!("ambient:{}", NEXT.fetch_add(1, Ordering::Relaxed));
    if let Some(dir) = config_dir.filter(|d| !d.trim().is_empty()) {
        let dir = PathBuf::from(dir);
        let probe = dir.clone();
        global().enter(&key_of(&dir), &owner, true, move || token_is_stale(&probe)).await;
    }
    AmbientGuard { owner }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn dir() -> PathBuf {
        PathBuf::from("/identities/a/claude")
    }

    fn stale_flag(v: bool) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(v))
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_spawn_on_an_expired_dir_goes_first_and_the_second_waits() {
        let w = Arc::new(Warmup::default());
        let stale = stale_flag(true);
        let (s1, s2) = (stale.clone(), stale.clone());
        w.enter(&dir(), "a", false, move || s1.load(Ordering::SeqCst)).await;

        let second = {
            let w = w.clone();
            tokio::spawn(async move { w.enter(&dir(), "b", false, move || s2.load(Ordering::SeqCst)).await })
        };
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!second.is_finished(), "b must wait while a holds the dir and the token is expired");

        assert!(w.release("a"));
        second.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiter_goes_once_the_token_is_fresh_though_the_owner_is_still_in_its_turn() {
        let w = Arc::new(Warmup::default());
        let stale = stale_flag(true);
        let s1 = stale.clone();
        w.enter(&dir(), "a", false, move || s1.load(Ordering::SeqCst)).await;

        let s2 = stale.clone();
        let second = {
            let w = w.clone();
            tokio::spawn(async move { w.enter(&dir(), "b", false, move || s2.load(Ordering::SeqCst)).await })
        };
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(!second.is_finished());
        stale.store(false, Ordering::SeqCst);
        tokio::time::sleep(POLL * 2).await;
        second.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_fresh_token_makes_nobody_wait() {
        let w = Warmup::default();
        for owner in ["a", "b", "c"] {
            w.enter(&dir(), owner, false, || false).await;
        }
        assert!(!w.release("a"), "nothing was taken");
    }

    #[tokio::test(start_paused = true)]
    async fn the_owner_entering_again_does_not_wait_for_itself() {
        let w = Warmup::default();
        w.enter(&dir(), "a", false, || true).await;
        timeout(Duration::from_millis(1), w.enter(&dir(), "a", false, || true)).await.expect("a respawn must not wait");
    }

    #[tokio::test(start_paused = true)]
    async fn an_owner_that_never_reports_back_frees_the_dir_after_warm_max() {
        let w = Arc::new(Warmup::default());
        w.enter(&dir(), "a", false, || true).await;
        w.started("a");
        let second = {
            let w = w.clone();
            tokio::spawn(async move { w.enter(&dir(), "b", false, || true).await })
        };
        tokio::time::sleep(WARM_MAX - Duration::from_secs(1)).await;
        assert!(!second.is_finished());
        tokio::time::sleep(Duration::from_secs(3)).await;
        second.await.unwrap();
        // b took it over, so a third now waits for b.
        let third = {
            let w = w.clone();
            tokio::spawn(async move { w.enter(&dir(), "c", false, || true).await })
        };
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(!third.is_finished());
        w.release("b");
        third.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_spawn_that_never_starts_its_process_frees_the_dir_after_the_grace() {
        let w = Arc::new(Warmup::default());
        w.enter(&dir(), "a", false, || true).await;
        let second = {
            let w = w.clone();
            tokio::spawn(async move { w.enter(&dir(), "b", false, || true).await })
        };
        tokio::time::sleep(GATE_GRACE - Duration::from_secs(1)).await;
        assert!(!second.is_finished());
        tokio::time::sleep(Duration::from_secs(3)).await;
        second.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn the_same_dir_spelled_two_ways_is_one_dir() {
        assert_eq!(key_of(Path::new("/identities/a/claude/")), key_of(Path::new("/identities/a/claude")));
        assert_eq!(key_of(Path::new(r"C:\Ids\a\claude\")), key_of(Path::new("C:/Ids/a/claude")));
        if cfg!(windows) {
            assert_eq!(key_of(Path::new("C:/IDS/a")), key_of(Path::new("c:/ids/A")));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn spawns_that_waited_start_a_stagger_apart() {
        let w = Arc::new(Warmup::default());
        let stale = stale_flag(true);
        let s0 = stale.clone();
        w.enter(&dir(), "a", false, move || s0.load(Ordering::SeqCst)).await;
        let mut waiters = Vec::new();
        for owner in ["b", "c", "d"] {
            let (w, s) = (w.clone(), stale.clone());
            waiters.push(tokio::spawn(async move {
                w.enter(&dir(), owner, false, move || s.load(Ordering::SeqCst)).await;
                Instant::now()
            }));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        stale.store(false, Ordering::SeqCst);
        let mut at = Vec::new();
        for h in waiters {
            at.push(h.await.unwrap());
        }
        at.sort();
        assert!(at[1] - at[0] >= STAGGER, "second start {:?} after the first", at[1] - at[0]);
        assert!(at[2] - at[1] >= STAGGER, "third start {:?} after the second", at[2] - at[1]);
    }

    #[tokio::test(start_paused = true)]
    async fn an_ambient_call_waits_for_the_first_turn_even_when_the_token_is_fresh_again() {
        let w = Arc::new(Warmup::default());
        let stale = stale_flag(true);
        let s1 = stale.clone();
        w.enter(&dir(), "a", false, move || s1.load(Ordering::SeqCst)).await;
        stale.store(false, Ordering::SeqCst);

        let ambient = {
            let w = w.clone();
            tokio::spawn(async move { w.enter(&dir(), "ambient:0", true, || false).await })
        };
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert!(!ambient.is_finished(), "the ambient call waits until a's turn ends");
        w.release("a");
        ambient.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn an_ambient_call_on_an_expired_dir_holds_it_against_a_spawn() {
        let w = Arc::new(Warmup::default());
        w.enter(&dir(), "ambient:0", true, || true).await;
        let spawn = {
            let w = w.clone();
            tokio::spawn(async move { w.enter(&dir(), "a", false, || true).await })
        };
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!spawn.is_finished());
        w.release("ambient:0");
        spawn.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_dropped_while_it_waits_leaves_no_dir_held() {
        let w = Arc::new(Warmup::default());
        let stale = stale_flag(true);
        let s0 = stale.clone();
        w.enter(&dir(), "a", false, move || s0.load(Ordering::SeqCst)).await;
        // b waits for a, then (a gone, token still expired) takes the dir, then staggers.
        let b = {
            let (w, s) = (w.clone(), stale.clone());
            tokio::spawn(async move { w.enter(&dir(), "b", false, move || s.load(Ordering::SeqCst)).await })
        };
        tokio::time::sleep(Duration::from_secs(1)).await;
        w.release("a");
        // Earlier waiters booked the next slots (b has not run yet), so b, once it takes the dir,
        // has a stagger to wait out.
        for _ in 0..3 {
            w.reserve_release(&dir(), Instant::now());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        b.abort();
        let _ = b.await;
        assert!(!w.release("b"), "aborting b already freed what it took");
    }

    #[tokio::test(start_paused = true)]
    async fn different_dirs_do_not_wait_for_each_other() {
        let w = Warmup::default();
        w.enter(Path::new("/identities/a/claude"), "a", false, || true).await;
        timeout(Duration::from_millis(1), w.enter(Path::new("/identities/b/claude"), "b", false, || true))
            .await
            .expect("another dir's token is its own");
    }

    fn write_creds(dir: &Path, expires_at_ms: i64, refresh: &str) {
        let body = serde_json::json!({"claudeAiOauth": {"accessToken": "x", "refreshToken": refresh, "expiresAt": expires_at_ms}});
        std::fs::write(dir.join(".credentials.json"), body.to_string()).unwrap();
    }

    #[test]
    fn staleness_reads_the_credentials_file() {
        let tmp = tempfile::tempdir().unwrap();
        let now = agentmux_common::time::now_ms();

        assert!(!token_is_stale(tmp.path()), "no file: not ours to wait on");
        write_creds(tmp.path(), now - 1_000, "r");
        assert!(token_is_stale(tmp.path()));
        write_creds(tmp.path(), now + 60_000, "r");
        assert!(token_is_stale(tmp.path()), "inside the CLI's own refresh margin");
        write_creds(tmp.path(), now + 3_600_000, "r");
        assert!(!token_is_stale(tmp.path()));
        write_creds(tmp.path(), now - 1_000, "");
        assert!(!token_is_stale(tmp.path()), "no refresh token: waiting cannot help");
    }

    #[tokio::test(start_paused = true)]
    async fn the_ambient_gate_with_no_dir_does_not_wait() {
        let g = timeout(Duration::from_millis(1), ambient_gate(None)).await.expect("no dir, no wait");
        drop(g);
        timeout(Duration::from_millis(1), ambient_gate(Some("  "))).await.expect("a blank dir is no dir");
    }
}
