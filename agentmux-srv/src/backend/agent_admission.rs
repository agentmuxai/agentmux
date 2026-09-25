// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One live instance per agent — the host tier
//! (`docs/specs/SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md`, Phase 1).
//!
//! An agent identity (its UID) may be driven by at most one process on this
//! host. Two layers:
//!
//! - [`acquire`] — the binding decision. A persistent pane claims the
//!   agent's [`LeaseStore`] lease right before its CLI process is spawned
//!   and holds it for that process's lifetime ([`HeldAgentLease`]): renewed
//!   on its own task, released on drop, and **fenced** — a holder that loses
//!   the lease (suspended past the TTL and reclaimed) is told through
//!   `on_lost` so it stops driving, and [`HeldAgentLease::verify`] lets it
//!   refuse a turn before starting one.
//! - [`check_before_spawn`] — the early, read-only check the async spawn
//!   paths run **before** they write anything shared on the agent's behalf
//!   (spec I9: in the 2026-09-25 incident the second instance broke the
//!   first by rebinding its account, before any spawn). It reads the lease
//!   and, for instances too old to take one, asks every other live
//!   AgentMux instance on this host whether it is running this UID.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::registry::{ClaimantInfo, Lease, LeaseError, LeaseHolder, LeaseStore};

/// The lease store under a shared agent registry root, `None` if there is
/// no registry or its lease directory can't be opened (logged).
pub fn lease_store_for(registry: Option<Arc<crate::registry::Registry>>) -> Option<Arc<LeaseStore>> {
    let registry = registry?;
    match LeaseStore::open(registry.root()) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::error!(error = %e, "agent_admission: cannot open the lease store — no single-instance check");
            None
        }
    }
}

/// What this instance records about itself in a lease it takes.
pub fn claimant_info() -> ClaimantInfo {
    ClaimantInfo {
        channel: crate::backend::reactive::registry::local_channel_id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// The refusal a user sees when `agent` is already running elsewhere.
pub fn denied_message(agent: &str, holder: Option<&LeaseHolder>) -> String {
    let agent = if agent.is_empty() { "This agent" } else { agent };
    let Some(h) = holder else {
        return format!(
            "{agent} is already running in another AgentMux instance on this host. \
             Close it there, then try again — an agent can only run in one place at a time."
        );
    };
    let mut place = Vec::new();
    if !h.channel.is_empty() {
        place.push(format!("channel {}", h.channel));
    }
    if !h.version.is_empty() {
        place.push(format!("v{}", h.version));
    }
    if h.pid != 0 {
        place.push(format!("pid {}", h.pid));
    }
    if h.acquired_at_ms > 0 {
        let mins = (agentmux_common::time::now_ms() - h.acquired_at_ms).max(0) / 60_000;
        place.push(format!("running for {mins} min"));
    }
    let place = if place.is_empty() { String::new() } else { format!(" ({})", place.join(", ")) };
    format!(
        "{agent} is already running in another AgentMux instance on this host{place}. \
         Close it there, then try again — an agent can only run in one place at a time."
    )
}

/// A lease held for the lifetime of one CLI process. Dropping it stops the
/// renewal task and releases the lease.
pub struct HeldAgentLease {
    store: Arc<LeaseStore>,
    lease: Lease,
    agent: String,
    lost: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl HeldAgentLease {
    /// The pre-turn fence check (spec I4). `Err` means another instance now
    /// holds this agent: do not start the turn. A transient I/O error is not
    /// a loss — the renewal task decides that — so it does not refuse.
    pub fn verify(&self) -> Result<(), String> {
        if self.lost.load(Ordering::SeqCst) {
            return Err(lost_message(&self.agent, None));
        }
        match self.store.verify(&self.lease) {
            Ok(()) => Ok(()),
            Err(LeaseError::HeldByOther { holder, .. }) => {
                self.lost.store(true, Ordering::SeqCst);
                Err(lost_message(&self.agent, holder.as_ref()))
            }
            Err(LeaseError::Io(e)) => {
                tracing::warn!(agent = %self.agent, error = %e, "agent lease: verify failed (io) — not treating as lost");
                Ok(())
            }
        }
    }
}

impl Drop for HeldAgentLease {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Err(e) = self.store.release(&self.lease) {
            tracing::warn!(
                agent = %self.agent,
                instance_id = %self.lease.instance_id(),
                error = %e,
                "agent lease: release failed (self-heals via TTL expiry)"
            );
        }
    }
}

fn lost_message(agent: &str, holder: Option<&LeaseHolder>) -> String {
    let agent = if agent.is_empty() { "This agent" } else { agent };
    match holder {
        Some(_) => format!(
            "{agent} was taken over by another AgentMux instance on this host — {}",
            denied_message(agent, holder)
        ),
        None => format!("{agent} lost its single-instance lease; it will not start another turn here."),
    }
}

/// Claim `uid` for this process's `block_id`, and keep it renewed until the
/// returned handle is dropped. `on_lost` runs (once, off the caller's
/// thread) if the lease is found to belong to someone else — the holder must
/// then stop driving the agent.
///
/// `Err` is the user-facing refusal. The host tier fails closed (spec D2):
/// an I/O error claiming the lease refuses rather than running unguarded,
/// after one short retry for a transient sharing violation.
pub fn acquire(
    store: &Arc<LeaseStore>,
    uid: &str,
    agent: &str,
    boot_id: &Arc<str>,
    block_id: &str,
    session_id_hint: Option<&str>,
    on_lost: impl Fn(String) + Send + Sync + 'static,
) -> Result<HeldAgentLease, String> {
    let info = claimant_info();
    let mut result = store.claim_as(uid, boot_id, block_id, session_id_hint, &info);
    if matches!(result, Err(LeaseError::Io(_))) {
        std::thread::sleep(std::time::Duration::from_millis(100));
        result = store.claim_as(uid, boot_id, block_id, session_id_hint, &info);
    }
    let lease = match result {
        Ok(lease) => lease,
        Err(LeaseError::HeldByOther { holder, owner_boot_id, age_ms, .. }) => {
            tracing::warn!(
                agent,
                uid,
                block_id,
                owner_boot_id = %owner_boot_id,
                holder_block = holder.as_ref().map(|h| h.block_id.as_str()).unwrap_or(""),
                holder_channel = holder.as_ref().map(|h| h.channel.as_str()).unwrap_or(""),
                age_ms,
                "agent_admission.denied: agent is live in another instance"
            );
            return Err(denied_message(agent, holder.as_ref()));
        }
        Err(LeaseError::Io(e)) => {
            tracing::error!(agent, uid, block_id, error = %e, "agent_admission.unknown: lease claim failed (io) — refusing");
            return Err(format!(
                "Could not confirm that {} isn't already running elsewhere (lease file error: {e}). \
                 Refusing to start it twice.",
                if agent.is_empty() { "this agent" } else { agent }
            ));
        }
    };
    tracing::info!(agent, uid, block_id, epoch = lease.epoch(), "agent_admission.granted");

    let held = HeldAgentLease {
        store: Arc::clone(store),
        lease: lease.clone(),
        agent: agent.to_string(),
        lost: Arc::new(AtomicBool::new(false)),
        stopped: Arc::new(AtomicBool::new(false)),
    };
    spawn_renewal(
        Arc::clone(store),
        lease,
        agent.to_string(),
        Arc::clone(&held.lost),
        Arc::clone(&held.stopped),
        on_lost,
    );
    Ok(held)
}

fn spawn_renewal(
    store: Arc<LeaseStore>,
    lease: Lease,
    agent: String,
    lost: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    on_lost: impl Fn(String) + Send + Sync + 'static,
) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        // No runtime (unit tests that spawn synchronously): the lease still
        // guards the claim and is released on drop; it just isn't renewed.
        return;
    };
    handle.spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(crate::registry::RENEW_INTERVAL_MS));
        // A laptop waking from sleep fires one tick, not a burst.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await; // the first tick is immediate; the claim just wrote the file
        loop {
            interval.tick().await;
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            let (s, l) = (Arc::clone(&store), lease.clone());
            let renewed = tokio::task::spawn_blocking(move || s.renew(&l)).await;
            // Checked again after the blocking renew: a release that ran
            // meanwhile must not be reported as a loss.
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            match renewed {
                Ok(Ok(())) => {}
                Ok(Err(LeaseError::HeldByOther { holder, owner_boot_id, .. })) => {
                    lost.store(true, Ordering::SeqCst);
                    tracing::error!(
                        agent = %agent,
                        instance_id = %lease.instance_id(),
                        owner_boot_id = %owner_boot_id,
                        "agent_admission.fenced: lease lost to another instance — stopping this one"
                    );
                    on_lost(lost_message(&agent, holder.as_ref()));
                    break;
                }
                Ok(Err(LeaseError::Io(e))) => {
                    tracing::warn!(agent = %agent, error = %e, "agent lease: renew failed (io) — will retry");
                }
                Err(join) => {
                    tracing::warn!(agent = %agent, error = %join, "agent lease: renew task panicked — will retry");
                }
            }
        }
    });
}

/// A live instance on this host that is running the same agent UID but
/// took no lease (it predates Phase 1) — found by asking it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyHolder {
    pub channel: String,
    pub local_url: String,
    pub block_id: String,
}

/// The early, read-only admission check (see module doc). `Err` is the
/// user-facing refusal; callers must run it before any shared write.
pub async fn check_before_spawn(
    store: Option<Arc<LeaseStore>>,
    uid: &str,
    agent: &str,
    own_boot_id: &str,
) -> Result<(), String> {
    if uid.is_empty() {
        return Ok(());
    }
    if let Some(store) = store {
        let (u, b) = (uid.to_string(), own_boot_id.to_string());
        match tokio::task::spawn_blocking(move || store.live_holder_other_than(&u, &b)).await {
            Ok(Ok(Some(holder))) => {
                tracing::warn!(agent, uid, holder_channel = %holder.channel, "agent_admission.denied (early check)");
                return Err(denied_message(agent, Some(&holder)));
            }
            Ok(Ok(None)) => {}
            Ok(Err(e)) => tracing::warn!(agent, uid, error = %e, "agent_admission: early lease read failed — the spawn-time claim still decides"),
            Err(e) => tracing::warn!(agent, uid, error = %e, "agent_admission: early lease read panicked"),
        }
    }
    if let Some(legacy) = probe_other_instances(uid).await {
        tracing::warn!(
            agent,
            uid,
            holder_channel = %legacy.channel,
            holder_url = %legacy.local_url,
            holder_block = %legacy.block_id,
            "agent_admission.compat_probe_hit: agent is live in an instance that takes no lease"
        );
        let holder = LeaseHolder {
            owner_boot_id: String::new(),
            block_id: legacy.block_id,
            pid: 0,
            channel: legacy.channel,
            version: String::new(),
            hostname: String::new(),
            epoch: 0,
            acquired_at_ms: 0,
            renewed_at_ms: 0,
        };
        return Err(denied_message(agent, Some(&holder)));
    }
    Ok(())
}

static PROBE_CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1_500))
        .build()
        .unwrap_or_default()
});

/// Ask every other live AgentMux instance on this host (from the
/// host-global cross-channel registry) whether it has `uid` registered.
/// Matched by UID, never by name (display names are not unique — Codex P2
/// on #3730). An instance too old to report UIDs, or one that doesn't
/// answer, is not evidence either way and is skipped (logged).
pub async fn probe_other_instances(uid: &str) -> Option<LegacyHolder> {
    let shared = crate::registry::resolve_shared_reactive_dir()?;
    let own_channel = crate::backend::reactive::registry::local_channel_id();
    let own_pid = std::process::id();
    let entries = tokio::task::spawn_blocking(move || {
        crate::backend::reactive::registry::list_all_shared(&shared)
    })
    .await
    .ok()?;
    let mut seen = std::collections::HashSet::new();
    let others: Vec<_> = entries
        .into_iter()
        .filter(|e| e.channel != own_channel && e.pid != own_pid && !e.local_url.is_empty())
        .filter(|e| crate::backend::reactive::registry::pid_alive(e.pid))
        .filter(|e| seen.insert(e.local_url.clone()))
        .collect();
    let probes = others.into_iter().map(|e| async move {
        let found = registered_uid_blocks(&e.local_url, &e.auth_key, uid).await;
        (e, found)
    });
    for (entry, found) in futures_util::future::join_all(probes).await {
        match found {
            Ok(Some(block_id)) => {
                return Some(LegacyHolder {
                    channel: entry.channel,
                    local_url: entry.local_url,
                    block_id,
                })
            }
            Ok(None) => {}
            Err(why) => tracing::debug!(
                channel = %entry.channel,
                url = %entry.local_url,
                why,
                "agent_admission: other instance not probeable — skipped"
            ),
        }
    }
    None
}

/// `GET <local_url>/agentmux/reactive/agents` and return the block of the
/// first registration whose `uid` equals `uid`.
async fn registered_uid_blocks(local_url: &str, auth_key: &str, uid: &str) -> Result<Option<String>, String> {
    let mut req = PROBE_CLIENT.get(format!("{local_url}/agentmux/reactive/agents"));
    if !auth_key.is_empty() {
        req = req.header("X-AuthKey", auth_key);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let regs: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(find_uid_block(&regs, uid))
}

/// The block of the first registration in `regs` carrying `uid`.
fn find_uid_block(regs: &[serde_json::Value], uid: &str) -> Option<String> {
    regs.iter().find_map(|r| {
        let r_uid = r.get("uid").and_then(|v| v.as_str())?;
        (r_uid.trim().eq_ignore_ascii_case(uid.trim())).then(|| {
            r.get("block_id").and_then(|v| v.as_str()).unwrap_or_default().to_string()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(ttl_ms: u64) -> (tempfile::TempDir, Arc<LeaseStore>) {
        let tmp = tempfile::tempdir().unwrap();
        let s = LeaseStore::open_with_ttl(tmp.path(), ttl_ms).unwrap();
        (tmp, Arc::new(s))
    }

    fn boot(id: &str) -> Arc<str> {
        Arc::from(id)
    }

    #[test]
    fn a_second_instance_is_refused_with_the_holder_named() {
        let (_tmp, s) = store(60_000);
        let _held = acquire(&s, "uid-1", "Agent3", &boot("boot-a"), "block-a", None, |_| {}).unwrap();
        let err = acquire(&s, "uid-1", "Agent3", &boot("boot-b"), "block-b", None, |_| {})
            .err()
            .expect("second instance must be refused");
        assert!(err.starts_with("Agent3 is already running in another AgentMux instance"), "{err}");
        assert!(err.contains(&format!("pid {}", std::process::id())), "{err}");
        assert!(err.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))), "{err}");
    }

    #[test]
    fn dropping_the_handle_releases_the_agent() {
        let (_tmp, s) = store(60_000);
        let held = acquire(&s, "uid-1", "Agent3", &boot("boot-a"), "block-a", None, |_| {}).unwrap();
        drop(held);
        assert!(acquire(&s, "uid-1", "Agent3", &boot("boot-b"), "block-b", None, |_| {}).is_ok());
    }

    #[test]
    fn verify_refuses_the_next_turn_once_the_lease_is_taken_over() {
        let (_tmp, s) = store(50);
        let held = acquire(&s, "uid-1", "Agent3", &boot("boot-a"), "block-a", None, |_| {}).unwrap();
        assert!(held.verify().is_ok());
        // Holder "suspended" past the TTL; another instance takes over.
        std::thread::sleep(std::time::Duration::from_millis(1_000));
        let _new = acquire(&s, "uid-1", "Agent3", &boot("boot-b"), "block-b", None, |_| {}).unwrap();
        let err = held.verify().unwrap_err();
        assert!(err.contains("taken over"), "{err}");
        // And it stays refused without re-reading.
        assert!(held.verify().is_err());
    }

    /// The old holder's drop must not release the new holder's lease.
    #[test]
    fn a_fenced_holder_dropping_does_not_free_the_new_holder() {
        let (_tmp, s) = store(50);
        let held = acquire(&s, "uid-1", "Agent3", &boot("boot-a"), "block-a", None, |_| {}).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1_000));
        let new = acquire(&s, "uid-1", "Agent3", &boot("boot-b"), "block-b", None, |_| {}).unwrap();
        drop(held);
        assert!(new.verify().is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_renewal_task_reports_a_takeover_through_on_lost() {
        let (tmp, s) = store(60_000);
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let tx = std::sync::Mutex::new(tx);
        let held = acquire(&s, "uid-1", "Agent3", &boot("boot-a"), "block-a", None, move |msg| {
            let _ = tx.lock().unwrap().send(msg);
        })
        .unwrap();
        // Simulate a takeover: another owner's lease file replaces ours.
        let other = LeaseStore::open_with_ttl(tmp.path(), 60_000).unwrap();
        let path = tmp.path().join("leases").join("uid-1.lease.json");
        std::fs::remove_file(&path).unwrap();
        other.claim("uid-1", &boot("boot-b"), "block-b", None).unwrap();
        let msg = tokio::task::spawn_blocking(move || {
            rx.recv_timeout(std::time::Duration::from_millis(crate::registry::RENEW_INTERVAL_MS * 3))
        })
        .await
        .unwrap()
        .expect("on_lost must fire within a couple of renew intervals");
        assert!(msg.contains("taken over"), "{msg}");
        assert!(held.verify().is_err());
    }

    #[tokio::test]
    async fn the_early_check_refuses_while_another_instance_holds_the_lease() {
        let (_tmp, s) = store(60_000);
        let _held = acquire(&s, "uid-1", "Agent3", &boot("boot-a"), "block-a", None, |_| {}).unwrap();
        let err = check_before_spawn(Some(Arc::clone(&s)), "uid-1", "Agent3", "boot-b")
            .await
            .unwrap_err();
        assert!(err.contains("already running"), "{err}");
        // The holder itself passes.
        assert!(check_before_spawn(Some(s), "uid-1", "Agent3", "boot-a").await.is_ok());
    }

    #[test]
    fn the_probe_matches_by_uid_never_by_name() {
        let regs = vec![
            serde_json::json!({"agent_id": "Agent3", "block_id": "b-other", "uid": "someone-else"}),
            serde_json::json!({"agent_id": "Agent3", "block_id": "b-legacy", "uid": null}),
            serde_json::json!({"agent_id": "Renamed", "block_id": "b-mine", "uid": "UID-1"}),
        ];
        assert_eq!(find_uid_block(&regs, "uid-1").as_deref(), Some("b-mine"));
        assert_eq!(find_uid_block(&regs[..2], "uid-1"), None, "a same-named agent with another or no UID is not a holder");
    }

    #[test]
    fn denied_message_without_details_still_says_what_to_do() {
        let m = denied_message("", None);
        assert!(m.starts_with("This agent is already running"), "{m}");
        assert!(m.contains("Close it there"), "{m}");
    }
}
