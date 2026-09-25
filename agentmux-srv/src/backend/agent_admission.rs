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
    /// Set once, by whichever notices first (the renewal task or a pre-turn
    /// `verify`): the refusal every later turn repeats — so the second and
    /// later refusals still name who took the agent over (ReAgent P2 on
    /// #3738), not a detail-less fallback.
    lost: Arc<LostReason>,
    stopped: Arc<AtomicBool>,
    /// When the lease was last confirmed ours (claim, renew or verify), in
    /// ms. [`RecordFence`] trusts the in-memory state only while this is
    /// within the TTL.
    last_ok_ms: Arc<std::sync::atomic::AtomicI64>,
}

/// The first loss message, kept for every later refusal.
#[derive(Default)]
struct LostReason(std::sync::Mutex<Option<String>>);

impl LostReason {
    fn get(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Record `msg` unless a reason is already recorded; return the one kept.
    fn set_first(&self, msg: String) -> String {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(msg).clone()
    }
}

impl HeldAgentLease {
    /// The ownership generation this lease was granted under.
    pub fn epoch(&self) -> u64 {
        self.lease.epoch()
    }

    /// A cheap handle the agent's output writer checks before each append
    /// to the agent's shared record (spec §4.3, Phase 3).
    pub fn record_fence(&self) -> RecordFence {
        RecordFence {
            store: Arc::clone(&self.store),
            lease: self.lease.clone(),
            agent: self.agent.clone(),
            lost: Arc::clone(&self.lost),
            last_ok_ms: Arc::clone(&self.last_ok_ms),
        }
    }

    /// The pre-turn fence check (spec I4). `Err` means another instance now
    /// holds this agent: do not start the turn. A transient I/O error is not
    /// a loss — the renewal task decides that — so it does not refuse.
    pub fn verify(&self) -> Result<(), String> {
        if let Some(why) = self.lost.get() {
            return Err(why);
        }
        match self.store.verify(&self.lease) {
            Ok(()) => {
                self.last_ok_ms.store(agentmux_common::time::now_ms(), Ordering::SeqCst);
                Ok(())
            }
            Err(LeaseError::HeldByOther { holder, .. }) => {
                Err(self.lost.set_first(lost_message(&self.agent, holder.as_ref())))
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
        unregister_by_name(&self.agent, &self.lost);
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
    if let Some(refusal) = handover_hold_refusal(uid, agent) {
        tracing::info!(agent, uid, block_id, "agent_admission.denied: inside a handover hold");
        return Err(refusal);
    }
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
    let on_lost: Arc<dyn Fn(String) + Send + Sync> = Arc::new(on_lost);

    let held = HeldAgentLease {
        store: Arc::clone(store),
        lease: lease.clone(),
        agent: agent.to_string(),
        lost: Arc::new(LostReason::default()),
        stopped: Arc::new(AtomicBool::new(false)),
        last_ok_ms: Arc::new(std::sync::atomic::AtomicI64::new(agentmux_common::time::now_ms())),
    };
    spawn_renewal(
        Arc::clone(store),
        lease,
        agent.to_string(),
        Arc::clone(&held.lost),
        Arc::clone(&held.stopped),
        Arc::clone(&held.last_ok_ms),
        {
            let on_lost = Arc::clone(&on_lost);
            move |why: String| on_lost(why)
        },
    );
    register_by_name(agent, &held.lost, on_lost);
    Ok(held)
}

fn spawn_renewal(
    store: Arc<LeaseStore>,
    lease: Lease,
    agent: String,
    lost: Arc<LostReason>,
    stopped: Arc<AtomicBool>,
    last_ok_ms: Arc<std::sync::atomic::AtomicI64>,
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
        let acquired_at_ms = agentmux_common::time::now_ms();
        let my_host = this_host_label();
        let mut ticks: u32 = 0;
        loop {
            interval.tick().await;
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            ticks = ticks.wrapping_add(1);
            // LAN tier (spec §4.4): two hosts can start the same agent at the
            // same moment, before either sees the other. Every
            // LAN_RECHECK_TICKS renewals, ask the LAN again; the loser of the
            // deterministic tie-break fences itself.
            if ticks % LAN_RECHECK_TICKS == 0 {
                if let Some(winner) = lan_holders(lease.instance_id())
                    .await
                    .into_iter()
                    .find(|p| peer_wins(p.acquired_at_ms, &p.hostname, acquired_at_ms, &my_host))
                {
                    if stopped.load(Ordering::SeqCst) {
                        break;
                    }
                    let why = lost.set_first(lan_lost_message(&agent, &winner));
                    tracing::error!(
                        agent = %agent,
                        instance_id = %lease.instance_id(),
                        peer_host = %winner.hostname,
                        "agent_admission.fenced: another computer on the LAN runs this agent and wins the tie-break — stopping this one"
                    );
                    on_lost(why);
                    break;
                }
            }
            let (s, l) = (Arc::clone(&store), lease.clone());
            let renewed = tokio::task::spawn_blocking(move || s.renew(&l)).await;
            // Checked again after the blocking renew: a release that ran
            // meanwhile must not be reported as a loss.
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            match renewed {
                Ok(Ok(())) => last_ok_ms.store(agentmux_common::time::now_ms(), Ordering::SeqCst),
                Ok(Err(LeaseError::HeldByOther { holder, owner_boot_id, .. })) => {
                    let why = lost.set_first(lost_message(&agent, holder.as_ref()));
                    tracing::error!(
                        agent = %agent,
                        instance_id = %lease.instance_id(),
                        owner_boot_id = %owner_boot_id,
                        "agent_admission.fenced: lease lost to another instance — stopping this one"
                    );
                    on_lost(why);
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
#[derive(Clone, PartialEq, Eq)]
pub struct LegacyHolder {
    pub channel: String,
    pub local_url: String,
    pub block_id: String,
    /// That srv's full auth key, from its shared-registry entry — what a
    /// takeover request authenticates with. Never logged.
    pub auth_key: String,
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
    if let Some(refusal) = handover_hold_refusal(uid, agent) {
        return Err(refusal);
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
    if let Some(where_) = crate::muxbus::wan_lease::held_elsewhere(agent) {
        tracing::warn!(agent, uid, holder = %where_, "agent_admission.denied: the muxbus relay says another instance holds this agent");
        return Err(wan_denied_message(agent, &where_));
    }
    if let Some(peer) = lan_holders(uid).await.into_iter().next() {
        tracing::warn!(agent, uid, peer_host = %peer.hostname, "agent_admission.denied: agent is live on a LAN peer");
        return Err(lan_denied_message(agent, &peer));
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
                    auth_key: entry.auth_key,
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

// ── Phase 5: WAN tier — the relay decides (spec §4.5) ──────────────────

type LostCallback = Arc<dyn Fn(String) + Send + Sync>;

/// The held leases of this process by agent **name** (lower-cased) — the key
/// the muxbus relay and its pending queue use — so the WAN side can fence the
/// local holder when the relay says another instance holds the agent.
static HELD_BY_NAME: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, (Arc<LostReason>, LostCallback)>>> =
    std::sync::LazyLock::new(Default::default);

fn register_by_name(agent: &str, lost: &Arc<LostReason>, on_lost: LostCallback) {
    if agent.trim().is_empty() {
        return;
    }
    HELD_BY_NAME
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(agent.trim().to_lowercase(), (Arc::clone(lost), on_lost));
}

fn unregister_by_name(agent: &str, lost: &Arc<LostReason>) {
    let mut map = HELD_BY_NAME.lock().unwrap_or_else(|e| e.into_inner());
    let key = agent.trim().to_lowercase();
    // Only our own entry: a newer lease for the same name may have replaced it.
    if map.get(&key).is_some_and(|(l, _)| Arc::ptr_eq(l, lost)) {
        map.remove(&key);
    }
}

/// The relay says another instance holds `agent_name` (WAN tier): fence this
/// process's holder of that agent, if any — record the loss (the pre-turn
/// fence refuses its next turn) and stop its CLI process. `true` when a
/// local holder was fenced.
pub fn fence_agent_named(agent_name: &str, where_: &str) -> bool {
    let entry = HELD_BY_NAME
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&agent_name.trim().to_lowercase())
        .cloned();
    let Some((lost, on_lost)) = entry else { return false };
    if lost.get().is_some() {
        return false;
    }
    let why = lost.set_first(wan_lost_message(agent_name, where_));
    tracing::error!(agent = agent_name, holder = where_, "agent_admission.fenced: the muxbus relay says another instance holds this agent — stopping this one");
    on_lost(why);
    true
}

fn wan_place(where_: &str) -> String {
    if where_.is_empty() { String::new() } else { format!(" ({where_})") }
}

/// The refusal when the relay says another instance runs the agent. Carries
/// the host refusal's marker, so it classifies as `LiveElsewhere`.
pub fn wan_denied_message(agent: &str, where_: &str) -> String {
    let agent = if agent.is_empty() { "This agent" } else { agent };
    format!(
        "{agent} is already running in another AgentMux instance on another computer{}, according to AgentMux cloud. \
         Close it there, then try again — an agent can only run in one place at a time.",
        wan_place(where_)
    )
}

fn wan_lost_message(agent: &str, where_: &str) -> String {
    let agent = if agent.is_empty() { "This agent" } else { agent };
    format!(
        "{agent} was taken over by another AgentMux instance on another computer{}, according to AgentMux cloud, which \
         had it first. It will not start another turn here.",
        wan_place(where_)
    )
}

// ── Phase 4: LAN tier — detect and yield (spec §4.4) ───────────────────

/// Renewals between LAN re-checks: 6 × RENEW_INTERVAL_MS = 30 s.
const LAN_RECHECK_TICKS: u32 = 6;

/// Start times closer than this are "simultaneous": clocks on two hosts are
/// not compared finer than this; the hostname decides instead (§4.4).
pub const LAN_SKEW_BUDGET_MS: i64 = 10_000;

/// How long one LAN peer gets to answer.
const LAN_QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1_500);

static LAN_DISCOVERY: std::sync::OnceLock<Arc<crate::backend::lan_discovery::LanDiscoveryController>> =
    std::sync::OnceLock::new();

/// Called once at bootstrap: gives admission the LAN peer list. Without it
/// (tests, LAN discovery off) the LAN tier is skipped.
pub fn set_lan_discovery(lan: Arc<crate::backend::lan_discovery::LanDiscoveryController>) {
    let _ = LAN_DISCOVERY.set(lan);
}

/// A LAN peer's answer to `GET /agentmux/agent/holding?uid=…`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerHolding {
    pub held: bool,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub acquired_at_ms: i64,
}

/// This host's label, as a LAN peer reports it and as the tie-break compares.
fn this_host_label() -> String {
    crate::backend::reactive::registry::local_host_label()
}

/// What this host answers to a LAN peer asking about `uid`: held when any
/// instance on this host holds a live lease for it.
pub fn local_holding(store: Option<&Arc<LeaseStore>>, uid: &str) -> PeerHolding {
    let holder = store.and_then(|s| s.live_holder_other_than(uid, "").ok().flatten());
    match holder {
        Some(h) => PeerHolding {
            held: true,
            hostname: this_host_label(),
            channel: h.channel,
            version: h.version,
            acquired_at_ms: h.acquired_at_ms,
        },
        None => PeerHolding { hostname: this_host_label(), ..Default::default() },
    }
}

/// Every reachable LAN peer **on another host** that says it holds `uid`.
/// Unreachable or older peers (no route) are skipped: not evidence either
/// way (D2, fail open). Answers from this host are dropped: another AgentMux
/// instance here also shows up as a LAN peer, but it reads this host's own
/// lease store, so it would report this process's own lease back — the
/// host tier already governs this host.
pub async fn lan_holders(uid: &str) -> Vec<PeerHolding> {
    let Some(lan) = LAN_DISCOVERY.get() else { return Vec::new() };
    let peers = lan.get_instances();
    if peers.is_empty() || uid.is_empty() {
        return Vec::new();
    }
    let queries = peers.into_iter().map(|p| async move {
        let url = format!("http://{}:{}/agentmux/agent/holding", p.address, p.port);
        let req = PROBE_CLIENT
            .get(url)
            .query(&[("uid", uid)])
            .header("X-AuthKey", p.auth_key.as_str())
            .timeout(LAN_QUERY_TIMEOUT);
        match req.send().await {
            Ok(r) if r.status().is_success() => r.json::<PeerHolding>().await.ok(),
            _ => None,
        }
    });
    futures_util::future::join_all(queries)
        .await
        .into_iter()
        .flatten()
        .filter(|h| h.held && !is_this_host(&h.hostname))
        .collect()
}

fn is_this_host(hostname: &str) -> bool {
    hostname.trim().eq_ignore_ascii_case(&this_host_label())
}

/// The LAN tie-break (§4.4): does a peer that started the agent at
/// `peer_at` on `peer_host` win over this host's claim at `mine_at`? The
/// earlier start wins; starts within the skew budget are simultaneous and
/// the lower hostname wins. Deterministic and symmetric: exactly one side
/// wins for any pair of distinct hosts.
pub fn peer_wins(peer_at: i64, peer_host: &str, mine_at: i64, my_host: &str) -> bool {
    if (peer_at - mine_at).abs() < LAN_SKEW_BUDGET_MS {
        peer_host < my_host
    } else {
        peer_at < mine_at
    }
}

fn lan_place(peer: &PeerHolding) -> String {
    let mut place = Vec::new();
    if !peer.hostname.is_empty() {
        place.push(format!("computer {}", peer.hostname));
    }
    if !peer.channel.is_empty() {
        place.push(format!("channel {}", peer.channel));
    }
    if !peer.version.is_empty() {
        place.push(format!("v{}", peer.version));
    }
    if place.is_empty() { String::new() } else { format!(" ({})", place.join(", ")) }
}

/// The refusal when a LAN peer runs the agent. Carries the same marker as
/// the host refusal, so it classifies as `LiveElsewhere`.
pub fn lan_denied_message(agent: &str, peer: &PeerHolding) -> String {
    let agent = if agent.is_empty() { "This agent" } else { agent };
    format!(
        "{agent} is already running in another AgentMux instance on another computer on your network{}. \
         Close it there, then try again — an agent can only run in one place at a time.",
        lan_place(peer)
    )
}

fn lan_lost_message(agent: &str, peer: &PeerHolding) -> String {
    let agent = if agent.is_empty() { "This agent" } else { agent };
    format!(
        "{agent} was taken over by another AgentMux instance on another computer on your network{}, which started it \
         first. It will not start another turn here.",
        lan_place(peer)
    )
}

// ── Phase 2: takeover (spec §4.6) ────────────────────────────────────────

/// Wording every single-live-instance refusal contains (lower-cased), so
/// the failure classifier and callers can recognise one without a type.
///
/// Only refusals that mean *another instance holds the agent*. `acquire`'s
/// lease-file I/O failure ("… Refusing to start it twice.") is deliberately
/// not one: it is a local, possibly transient error, and classifying it as
/// "running in another instance" would offer Take over instead of Retry
/// (ReAgent P1 on #3742).
pub const REFUSAL_MARKERS: [&str; 3] = [
    "is already running in another agentmux instance",
    "was taken over by another agentmux instance",
    "lost its single-instance lease",
];

/// Is `message` a single-live-instance refusal (any of this module's
/// user-facing refusal texts)?
pub fn is_admission_refusal(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    REFUSAL_MARKERS.iter().any(|k| m.contains(k))
}

/// How long, after this srv hands an agent over to another instance, it
/// refuses to claim that agent again on its own. Covers the gap between the
/// holder's process exiting and the requester claiming, in which a jekt
/// delivered here would otherwise respawn the agent and win it straight
/// back. A takeover the user starts from this srv clears it.
pub const HANDOVER_HOLD: std::time::Duration = std::time::Duration::from_secs(30);

/// One entry per agent this srv handed over: until when, and to whom (for
/// the refusal text). A single map, so an expired hold goes away whole
/// (ReAgent P2 on #3742).
static HANDOVER_HOLDS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, String)>>> =
    std::sync::LazyLock::new(Default::default);

/// Record that this srv just handed `uid` over (see [`HANDOVER_HOLD`]).
pub fn hold_after_handover(uid: &str, to: &str) {
    HANDOVER_HOLDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(uid.to_string(), (std::time::Instant::now() + HANDOVER_HOLD, to.to_string()));
}

/// Drop any handover hold on `uid` — the user is taking it back.
pub fn clear_handover_hold(uid: &str) {
    HANDOVER_HOLDS.lock().unwrap_or_else(|e| e.into_inner()).remove(uid);
}

/// `Some(refusal)` while `uid` is inside a handover hold on this srv. An
/// expired hold is removed here.
fn handover_hold_refusal(uid: &str, agent: &str) -> Option<String> {
    let mut holds = HANDOVER_HOLDS.lock().unwrap_or_else(|e| e.into_inner());
    let (until, to) = holds.get(uid)?.clone();
    if until <= std::time::Instant::now() {
        holds.remove(uid);
        return None;
    }
    let agent = if agent.is_empty() { "This agent" } else { agent };
    Some(format!(
        "{agent} was taken over by another AgentMux instance on this host{}. \
         Use Take over to bring it back here.",
        if to.is_empty() { String::new() } else { format!(" ({to})") }
    ))
}

/// The writer-side fence on the agent's shared record (spec §4.3, Phase 3).
///
/// Cheap in the common case: while the lease was confirmed ours within the
/// TTL, nobody can have reclaimed it, so appends go ahead on in-memory state
/// alone. After a longer gap — the holder was suspended, and its CLI and
/// output reader resume together with the renewal task — the first append
/// checks the lease file before writing, so output a reclaimed holder
/// flushes on waking never reaches the record the new holder is writing.
#[derive(Clone)]
pub struct RecordFence {
    store: Arc<LeaseStore>,
    lease: Lease,
    agent: String,
    lost: Arc<LostReason>,
    last_ok_ms: Arc<std::sync::atomic::AtomicI64>,
}

impl RecordFence {
    /// May this process append to the agent's shared record now?
    pub fn may_write(&self) -> bool {
        if self.lost.get().is_some() {
            return false;
        }
        let now = agentmux_common::time::now_ms();
        let age = now - self.last_ok_ms.load(Ordering::SeqCst);
        if age >= 0 && (age as u64) <= crate::registry::LEASE_TTL_MS {
            return true;
        }
        match self.store.verify(&self.lease) {
            Ok(()) => {
                self.last_ok_ms.store(now, Ordering::SeqCst);
                true
            }
            Err(LeaseError::HeldByOther { holder, .. }) => {
                self.lost.set_first(lost_message(&self.agent, holder.as_ref()));
                tracing::error!(
                    agent = %self.agent,
                    instance_id = %self.lease.instance_id(),
                    "agent_admission.fenced: output from a process that lost its lease kept out of the shared record"
                );
                false
            }
            // An I/O hiccup is not a loss (same rule as `verify`).
            Err(LeaseError::Io(_)) => true,
        }
    }
}

/// `zone` unless `fence` says this process may no longer write the agent's
/// shared record.
pub fn fenced_zone<'a>(zone: Option<&'a str>, fence: &Option<RecordFence>) -> Option<&'a str> {
    match fence {
        Some(f) if zone.is_some() && !f.may_write() => None,
        _ => zone,
    }
}

/// Where the instance currently running an agent can be reached.
#[derive(Clone, PartialEq, Eq)]
pub struct HolderEndpoint {
    pub channel: String,
    pub local_url: String,
    /// Never logged.
    pub auth_key: String,
}

impl std::fmt::Debug for HolderEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HolderEndpoint")
            .field("channel", &self.channel)
            .field("local_url", &self.local_url)
            .finish_non_exhaustive()
    }
}

/// Find the other instance on this host that runs `uid`: the lease holder
/// (its channel's live srv, from the shared registry) or — for an instance
/// that takes no lease — the compat probe's hit. `None` when nobody else
/// runs it, or the holder's srv can't be found.
pub async fn locate_holder(store: Option<Arc<LeaseStore>>, uid: &str, own_boot_id: &str) -> Option<HolderEndpoint> {
    if let Some(store) = store {
        let (u, b) = (uid.to_string(), own_boot_id.to_string());
        let holder = tokio::task::spawn_blocking(move || store.live_holder_other_than(&u, &b))
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten();
        if let Some(holder) = holder.filter(|h| !h.channel.is_empty()) {
            let shared = crate::registry::resolve_shared_reactive_dir()?;
            let channel = holder.channel.clone();
            let entries = tokio::task::spawn_blocking(move || {
                crate::backend::reactive::registry::list_all_shared(&shared)
            })
            .await
            .ok()?;
            if let Some(e) = entries.into_iter().find(|e| {
                e.channel == channel
                    && !e.local_url.is_empty()
                    && crate::backend::reactive::registry::pid_alive(e.pid)
            }) {
                return Some(HolderEndpoint { channel: e.channel, local_url: e.local_url, auth_key: e.auth_key });
            }
        }
    }
    probe_other_instances(uid).await.map(|l| HolderEndpoint {
        channel: l.channel,
        local_url: l.local_url,
        auth_key: l.auth_key,
    })
}

/// Ask the holder's srv to stop running `uid` so this instance can take it
/// (`POST /agentmux/agent/release`). `Err` is user-facing.
pub async fn request_release(holder: &HolderEndpoint, uid: &str, agent: &str) -> Result<(), String> {
    let me = claimant_info();
    let mut req = PROBE_CLIENT
        .post(format!("{}/agentmux/agent/release", holder.local_url))
        .json(&serde_json::json!({
            "uid": uid,
            "requested_by_channel": me.channel,
            "requested_by_version": me.version,
        }));
    if !holder.auth_key.is_empty() {
        req = req.header("X-AuthKey", &holder.auth_key);
    }
    let agent = if agent.is_empty() { "this agent" } else { agent };
    let resp = req.send().await.map_err(|e| {
        format!("Could not reach the AgentMux instance running {agent} (channel {}): {e}", holder.channel)
    })?;
    match resp.status() {
        s if s.is_success() => Ok(()),
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED => Err(format!(
            "The AgentMux instance running {agent} (channel {}) is too old to hand it over. \
             Close {agent} there, then try again.",
            holder.channel
        )),
        s => {
            let body = resp.text().await.unwrap_or_default();
            Err(format!("The instance running {agent} refused to hand it over ({s}): {body}"))
        }
    }
}

/// Wait until no other instance holds `uid` (lease and compat probe both
/// clear), polling. `Err` after `timeout`.
pub async fn wait_until_free(
    store: Option<Arc<LeaseStore>>,
    uid: &str,
    own_boot_id: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if locate_holder(store.clone(), uid, own_boot_id).await.is_none()
            && check_before_spawn(store.clone(), uid, "", own_boot_id).await.is_ok()
        {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "The other instance did not let go within {}s. Try again in a moment.",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
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
        // And it stays refused, with the same detail, without re-reading
        // (ReAgent P2 on #3738: later refusals used to drop the holder).
        assert_eq!(held.verify().unwrap_err(), err);
        assert!(err.contains(&format!("pid {}", std::process::id())), "{err}");
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

    /// Phase 3: the writer trusts in-memory state within the TTL…
    #[test]
    fn the_record_fence_passes_a_recently_confirmed_lease_without_io() {
        let (_tmp, s) = store(60_000);
        let held = acquire(&s, "uid-1", "Agent3", &boot("boot-a"), "block-a", None, |_| {}).unwrap();
        let fence = held.record_fence();
        assert!(fence.may_write());
        assert_eq!(fenced_zone(Some("agent:uid-1:current"), &Some(fence)), Some("agent:uid-1:current"));
        assert_eq!(fenced_zone(Some("z"), &None), Some("z"), "no lease, no fence");
    }

    /// …and after a gap longer than the TTL (a suspended holder) it checks
    /// the lease file before writing: a reclaimed holder's output is kept
    /// out of the shared record.
    #[test]
    fn the_record_fence_keeps_a_reclaimed_holders_output_out_after_a_gap() {
        let (tmp, s) = store(60_000);
        let held = acquire(&s, "uid-1", "Agent3", &boot("boot-a"), "block-a", None, |_| {}).unwrap();
        let fence = held.record_fence();
        // Suspended past the TTL…
        held.last_ok_ms.store(agentmux_common::time::now_ms() - crate::registry::LEASE_TTL_MS as i64 - 1_000, Ordering::SeqCst);
        // …while another instance took the agent over.
        std::fs::remove_file(tmp.path().join("leases").join("uid-1.lease.json")).unwrap();
        let _new = acquire(&s, "uid-1", "Agent3", &boot("boot-b"), "block-b", None, |_| {}).unwrap();
        assert!(!fence.may_write());
        assert_eq!(fenced_zone(Some("agent:uid-1:current"), &Some(fence.clone())), None);
        assert!(held.verify().unwrap_err().contains("taken over"), "the loss is recorded for the pre-turn fence too");
    }

    #[test]
    fn the_record_fence_confirms_and_resumes_when_the_lease_is_still_ours_after_a_gap() {
        let (_tmp, s) = store(60_000);
        let held = acquire(&s, "uid-1", "Agent3", &boot("boot-a"), "block-a", None, |_| {}).unwrap();
        let fence = held.record_fence();
        let stale = agentmux_common::time::now_ms() - crate::registry::LEASE_TTL_MS as i64 - 1_000;
        held.last_ok_ms.store(stale, Ordering::SeqCst);
        assert!(fence.may_write(), "nobody reclaimed it: still ours");
        assert!(held.last_ok_ms.load(Ordering::SeqCst) > stale, "confirmed again");
    }

    /// Phase 4 (§4.4): exactly one of two hosts wins, whatever the order
    /// the check runs in, and near-simultaneous starts fall to the hostname.
    #[test]
    fn the_lan_tie_break_has_exactly_one_winner() {
        let cases = [
            (1_000_000, "alpha", 1_050_000, "beta"), // alpha 50 s earlier
            (1_050_000, "alpha", 1_000_000, "beta"), // beta 50 s earlier
            (1_000_000, "beta", 1_003_000, "alpha"), // within the skew budget
            (1_000_000, "alpha", 1_000_000, "beta"), // identical
        ];
        for (a_at, a_host, b_at, b_host) in cases {
            let a_wins = peer_wins(a_at, a_host, b_at, b_host); // b asks: does a win?
            let b_wins = peer_wins(b_at, b_host, a_at, a_host); // a asks: does b win?
            assert!(a_wins ^ b_wins, "exactly one winner for {a_host}@{a_at} vs {b_host}@{b_at}");
        }
        assert!(peer_wins(1_000_000, "alpha", 1_050_000, "beta"), "the earlier start wins");
        assert!(peer_wins(1_003_000, "alpha", 1_000_000, "beta"), "within the budget the lower hostname wins");
    }

    #[test]
    fn a_lan_refusal_is_recognised_and_names_the_computer() {
        let peer = PeerHolding {
            held: true,
            hostname: "narko".into(),
            channel: "stable".into(),
            version: "0.58.0".into(),
            acquired_at_ms: 1,
        };
        let m = lan_denied_message("Agent3", &peer);
        assert!(m.contains("another computer on your network") && m.contains("computer narko"), "{m}");
        assert!(is_admission_refusal(&m));
        assert!(is_admission_refusal(&lan_lost_message("Agent3", &peer)));
        assert_eq!(
            crate::agents::failure::classify(None, None, &m, None).code,
            crate::agents::failure::FailureClass::LiveElsewhere
        );
    }

    #[test]
    fn this_host_reports_holding_only_a_live_lease() {
        let (_tmp, s) = store(60_000);
        assert!(!local_holding(Some(&s), "uid-1").held);
        let held = acquire(&s, "uid-1", "Agent3", &boot("boot-a"), "block-a", None, |_| {}).unwrap();
        let h = local_holding(Some(&s), "uid-1");
        assert!(h.held);
        assert_eq!(h.version, env!("CARGO_PKG_VERSION"));
        assert!(h.acquired_at_ms > 0);
        drop(held);
        assert!(!local_holding(Some(&s), "uid-1").held);
        assert!(!local_holding(None, "uid-1").held, "no lease store, nothing held");
    }

    /// A second AgentMux instance on this same host is also a LAN peer, and
    /// reports this host's leases — including this process's own. The LAN
    /// tier must not refuse on that.
    #[test]
    fn an_answer_from_this_host_is_not_a_lan_holder() {
        assert!(is_this_host(&this_host_label()));
        assert!(is_this_host(&this_host_label().to_uppercase()));
        assert!(!is_this_host("some-other-computer"));
    }

    #[tokio::test]
    async fn without_lan_discovery_the_lan_tier_is_skipped() {
        // LAN_DISCOVERY is never set in unit tests.
        assert!(lan_holders("uid-1").await.is_empty());
    }

    /// Phase 5: the WAN side fences the local holder by agent name, only its
    /// own entry, and only once.
    #[test]
    fn the_relay_can_fence_the_local_holder_by_agent_name() {
        let (_tmp, s) = store(60_000);
        let name = format!("Agent-{}", uuid::Uuid::new_v4());
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let tx = std::sync::Mutex::new(tx);
        let held = acquire(&s, "uid-w", &name, &boot("boot-a"), "block-a", None, move |m| {
            let _ = tx.lock().unwrap().send(m);
        })
        .unwrap();
        assert!(fence_agent_named(&name.to_lowercase(), "computer desk, channel stable"));
        let why = rx.try_recv().expect("on_lost ran");
        assert!(why.contains("taken over") && why.contains("computer desk"), "{why}");
        assert!(is_admission_refusal(&why));
        assert_eq!(held.verify().unwrap_err(), why, "the next turn is refused with the same reason");
        assert!(!fence_agent_named(&name, "again"), "fenced once");
        drop(held);
        assert!(!fence_agent_named(&name, "after release"), "no holder left to fence");
    }

    #[test]
    fn a_wan_refusal_is_recognised() {
        let m = wan_denied_message("Agent3", "computer desk, channel stable, v0.58.0");
        assert!(is_admission_refusal(&m), "{m}");
        assert_eq!(
            crate::agents::failure::classify(None, None, &m, None).code,
            crate::agents::failure::FailureClass::LiveElsewhere
        );
    }

    #[test]
    fn every_refusal_text_is_recognised_as_one() {
        let holder = LeaseHolder {
            owner_boot_id: "b".into(), block_id: "x".into(), pid: 1, channel: "c".into(), version: "v".into(),
            hostname: String::new(), epoch: 1, acquired_at_ms: 0, renewed_at_ms: 0,
        };
        for m in [
            denied_message("Agent3", Some(&holder)),
            denied_message("", None),
            lost_message("Agent3", Some(&holder)),
            lost_message("Agent3", None),
        ] {
            assert!(is_admission_refusal(&m), "not recognised: {m}");
        }
        assert!(!is_admission_refusal("failed to spawn persistent process: not found"));
    }

    /// ReAgent P1 on #3742: a local lease-file I/O error is not "running in
    /// another instance" — it must keep a Retry, not get Take over.
    #[test]
    fn a_lease_file_io_error_is_not_classified_as_live_elsewhere() {
        let io = "Could not confirm that Agent3 isn't already running elsewhere (lease file error: access denied). \
                  Refusing to start it twice.";
        assert!(!is_admission_refusal(io));
        let f = crate::agents::failure::classify(None, None, io, None);
        assert_ne!(f.code, crate::agents::failure::FailureClass::LiveElsewhere);
    }

    /// ReAgent P2 on #3742: an expired hold is removed whole.
    #[test]
    fn an_expired_handover_hold_is_removed() {
        let uid = format!("uid-{}", uuid::Uuid::new_v4());
        HANDOVER_HOLDS
            .lock()
            .unwrap()
            .insert(uid.clone(), (std::time::Instant::now() - std::time::Duration::from_secs(1), "to".into()));
        assert!(handover_hold_refusal(&uid, "Agent3").is_none());
        assert!(!HANDOVER_HOLDS.lock().unwrap().contains_key(&uid));
    }

    #[test]
    fn a_handover_hold_refuses_this_srv_until_cleared() {
        let (_tmp, s) = store(60_000);
        let uid = format!("uid-{}", uuid::Uuid::new_v4());
        hold_after_handover(&uid, "channel other, v9");
        let err = acquire(&s, &uid, "Agent3", &boot("boot-a"), "block-a", None, |_| {})
            .err()
            .expect("a handed-over agent is not re-claimed on its own");
        assert!(err.contains("was taken over") && err.contains("channel other"), "{err}");
        assert!(is_admission_refusal(&err));
        clear_handover_hold(&uid);
        assert!(acquire(&s, &uid, "Agent3", &boot("boot-a"), "block-a", None, |_| {}).is_ok());
    }

    #[test]
    fn holder_endpoint_debug_never_prints_the_auth_key() {
        let ep = HolderEndpoint { channel: "c".into(), local_url: "http://x".into(), auth_key: "sekrit".into() };
        assert!(!format!("{ep:?}").contains("sekrit"));
    }

    #[test]
    fn denied_message_without_details_still_says_what_to_do() {
        let m = denied_message("", None);
        assert!(m.starts_with("This agent is already running"), "{m}");
        assert!(m.contains("Close it there"), "{m}");
    }
}
