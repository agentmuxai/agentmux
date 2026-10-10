// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What this install says about itself beyond agent names, on the LAN fleet
//! feed, the UDP and mDNS records and the cloud presence record
//! (agentmux-mobile's SPEC_FLEET_HOST_TAGS_AND_CLOUD_HOSTS_2026_10_06 §5–6):
//! each agent's kind, how many channels this machine is running, and this
//! install's id. Also each agent's state and since when
//! (`backend::agent_state`, agentmux-mobile's
//! SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §3).
//!
//! The fleet feed's change detection calls [`FleetSource::observe`] once a
//! second. The LAN replies, which have no fleet feed in hand, read the
//! channel count it last saw ([`latest_channels_running`]) and
//! [`install_id`].

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::backend::agent_state::{AgentState, SinceTracker};
use crate::backend::fleet_feed::{AgentKind, FleetObservation};
use crate::backend::reactive::registry::AgentEntry;
use crate::backend::storage::store::Store;

/// The upper bound of `channels_running` on every wire.
pub const MAX_CHANNELS_RUNNING: u32 = 99;

/// How long a channel count is reused before the shared registry is read
/// again. The registry's own heartbeat is 20 s; reading its directory every
/// second would buy nothing.
const CHANNELS_REREAD: Duration = Duration::from_secs(5);

/// The count [`FleetSource::observe`] last saw; 0 before the first look.
static LATEST_CHANNELS_RUNNING: AtomicU32 = AtomicU32::new(0);

static INSTALL_ID: OnceLock<String> = OnceLock::new();

/// This install's WAN instance id (26 chars of lowercase base32, a hash of
/// the instance public key; not a secret), or `None` when this process has
/// no WAN identity store. The instance is minted once per channel and never
/// changes, so the first answer is kept.
pub fn install_id() -> Option<String> {
    if let Some(id) = INSTALL_ID.get() {
        return Some(id.clone());
    }
    let wan = crate::backend::storage::wan_identity::global()?;
    let instance = wan
        .instance_ensure(&crate::backend::reactive::registry::local_host_label())
        .ok()?;
    Some(INSTALL_ID.get_or_init(|| instance.instance_id).clone())
}

/// 1 for this channel plus every other channel of this machine with a live
/// entry in the host-global shared registry (`swarm_remote::other_channels`,
/// with its `HIDE_AFTER_MS`), clamped to 1..=[`MAX_CHANNELS_RUNNING`]. A
/// channel with no agent registered has no entry and is not counted.
pub fn count_channels_running(entries: &[AgentEntry], own_channel: &str, own_url: &str, now_ms: u64) -> u32 {
    let others = crate::backend::swarm_remote::other_channels(entries, own_channel, own_url, now_ms).len();
    u32::try_from(others)
        .unwrap_or(u32::MAX)
        .saturating_add(1)
        .clamp(1, MAX_CHANNELS_RUNNING)
}

/// The channel count the fleet feed last saw, for the UDP replies; `None`
/// before its first look.
pub fn latest_channels_running() -> Option<u32> {
    match LATEST_CHANNELS_RUNNING.load(Ordering::Relaxed) {
        0 => None,
        n => Some(n),
    }
}

/// Each agent's kind by name, from its block. An agent whose block can't be
/// read is left out (a peer then shows no tag rather than a guess).
pub fn kinds_by_name<'a>(
    agents: impl IntoIterator<Item = (&'a str, &'a str)>,
    kind_of_block: impl Fn(&str) -> Option<AgentKind>,
) -> BTreeMap<String, AgentKind> {
    agents
        .into_iter()
        .filter_map(|(name, block_id)| kind_of_block(block_id).map(|k| (name.to_string(), k)))
        .collect()
}

type AgentList = Box<dyn Fn() -> Vec<(String, String)> + Send + Sync>;
type KindOfBlock = Box<dyn Fn(&str) -> Option<AgentKind> + Send + Sync>;
type StateOfBlock = Box<dyn Fn(&str) -> Option<AgentState> + Send + Sync>;
type RegistryRead = Box<dyn Fn() -> Vec<AgentEntry> + Send + Sync>;

/// The fleet feed's view of this instance.
pub struct FleetSource {
    /// `(agent name, block id)` of every reachable agent.
    agents: AgentList,
    kind_of_block: KindOfBlock,
    /// The block's state now (`agent_state::current_state`); read on every
    /// look, unlike the kind.
    state_of_block: StateOfBlock,
    /// When each state began; a block no longer listed is forgotten.
    since: &'static SinceTracker,
    shared_registry: RegistryRead,
    own_channel: String,
    own_url: String,
    /// Kinds already read, by block id. A block's `agentMode` is fixed when
    /// the agent is created, so each block is read once while it is listed.
    kinds: parking_lot::Mutex<HashMap<String, AgentKind>>,
    channels: parking_lot::Mutex<Option<(Instant, u32)>>,
}

impl FleetSource {
    /// The live source: the reactive registry, the object store and the
    /// host-global shared registry.
    pub fn live(
        handler: &'static crate::backend::reactive::handler::ReactiveHandler,
        mstore: Arc<Store>,
        own_url: String,
    ) -> Self {
        let state_store = Arc::clone(&mstore);
        Self::new(
            Box::new(move || {
                handler
                    .list_agents()
                    .into_iter()
                    .map(|a| (a.agent_id, a.block_id))
                    .collect()
            }),
            Box::new(move |block_id| crate::backend::operator_config_seed::agent_kind_of_block(&mstore, block_id)),
            Box::new(move |block_id| crate::backend::agent_state::current_state(&state_store, block_id)),
            crate::backend::agent_state::global_tracker(),
            Box::new(|| {
                crate::registry::resolve_shared_reactive_dir()
                    .map(|dir| crate::backend::reactive::registry::list_all_shared(&dir))
                    .unwrap_or_default()
            }),
            crate::backend::reactive::registry::local_channel_id(),
            own_url,
        )
    }

    fn new(
        agents: AgentList,
        kind_of_block: KindOfBlock,
        state_of_block: StateOfBlock,
        since: &'static SinceTracker,
        shared_registry: RegistryRead,
        own_channel: String,
        own_url: String,
    ) -> Self {
        Self {
            agents,
            kind_of_block,
            state_of_block,
            since,
            shared_registry,
            own_channel,
            own_url,
            kinds: parking_lot::Mutex::new(HashMap::new()),
            channels: parking_lot::Mutex::new(None),
        }
    }

    /// One look. Blocking: reads the store and the registry directory.
    pub fn observe(&self) -> FleetObservation {
        let listed = (self.agents)();
        self.since.retain(|block_id| listed.iter().any(|(_, b)| b == block_id));
        let now_ms = agentmux_common::time::now_ms_u64();
        let agent_status = listed
            .iter()
            .filter_map(|(name, block_id)| {
                let status = self.since.record(block_id, (self.state_of_block)(block_id), now_ms)?;
                Some((name.clone(), status))
            })
            .collect();
        let agents = {
            let mut cache = self.kinds.lock();
            cache.retain(|block_id, _| listed.iter().any(|(_, b)| b == block_id));
            listed
                .into_iter()
                .map(|(name, block_id)| {
                    let kind = match cache.get(&block_id) {
                        Some(kind) => Some(*kind),
                        None => {
                            let kind = (self.kind_of_block)(&block_id);
                            if let Some(kind) = kind {
                                cache.insert(block_id, kind);
                            }
                            kind
                        }
                    };
                    (name, kind)
                })
                .collect()
        };
        FleetObservation {
            agents,
            channels_running: self.channels_running(),
            agent_status,
        }
    }

    fn channels_running(&self) -> u32 {
        let mut last = self.channels.lock();
        if let Some((at, n)) = *last {
            if at.elapsed() < CHANNELS_REREAD {
                return n;
            }
        }
        let n = count_channels_running(
            &(self.shared_registry)(),
            &self.own_channel,
            &self.own_url,
            agentmux_common::time::now_ms_u64(),
        );
        *last = Some((Instant::now(), n));
        LATEST_CHANNELS_RUNNING.store(n, Ordering::Relaxed);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const NOW: u64 = 1_000_000_000;

    fn entry(name: &str, channel: &str, age_ms: u64) -> AgentEntry {
        AgentEntry {
            agent_id: name.to_string(),
            local_url: format!("http://127.0.0.1/{channel}"),
            block_id: format!("block-{name}"),
            pid: 1,
            updated_at: NOW - age_ms,
            auth_key: String::new(),
            channel: channel.to_string(),
            registration_nonce: 0,
            jekt_public_key: String::new(),
            uid: String::new(),
            uid_public_key: String::new(),
        }
    }

    #[test]
    fn channels_running_counts_this_channel_and_live_others() {
        assert_eq!(count_channels_running(&[], "stable", "", NOW), 1);
        let entries = [
            entry("a", "stable", 0),
            entry("b", "dev", 0),
            entry("c", "dev", 0),
            entry("d", "local-main", 1_000),
            entry("e", "crashed", crate::backend::swarm_remote::HIDE_AFTER_MS + 1),
        ];
        assert_eq!(
            count_channels_running(&entries, "stable", "", NOW),
            3,
            "this one, dev and local-main; not the hidden one"
        );
        let many: Vec<_> = (0..150).map(|i| entry("x", &format!("ch-{i}"), 0)).collect();
        assert_eq!(count_channels_running(&many, "stable", "", NOW), MAX_CHANNELS_RUNNING);
    }

    #[test]
    fn kinds_leave_out_unreadable_blocks() {
        let kinds = kinds_by_name([("AgentX", "b1"), ("Camper", "b2"), ("Ghost", "b3")], |b| match b {
            "b1" => Some("container"),
            "b2" => Some("host"),
            _ => None,
        });
        assert_eq!(
            kinds,
            BTreeMap::from([("AgentX".to_string(), "container"), ("Camper".to_string(), "host")])
        );
    }

    #[test]
    fn a_block_is_read_once_while_listed_and_an_unreadable_one_is_retried() {
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = reads.clone();
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ready_read = ready.clone();
        let source = FleetSource::new(
            Box::new(|| vec![("AgentX".into(), "b1".into()), ("Late".into(), "b2".into())]),
            Box::new(move |b| {
                counted.fetch_add(1, Ordering::SeqCst);
                match b {
                    "b1" => Some("container"),
                    _ if ready_read.load(Ordering::SeqCst) => Some("host"),
                    _ => None,
                }
            }),
            Box::new(|_| None),
            tracker(),
            Box::new(Vec::new),
            "stable".into(),
            String::new(),
        );
        let first = source.observe();
        assert_eq!(
            first.agents,
            vec![("AgentX".to_string(), Some("container")), ("Late".to_string(), None)]
        );
        assert_eq!(first.channels_running, 1);
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        source.observe();
        assert_eq!(reads.load(Ordering::SeqCst), 3, "b1 cached, b2 retried");
        ready.store(true, Ordering::SeqCst);
        assert_eq!(source.observe().agents[1], ("Late".to_string(), Some("host")));
        source.observe();
        assert_eq!(reads.load(Ordering::SeqCst), 4, "both cached now");
    }

    /// A tracker of the test's own, not the live one the names route shares.
    fn tracker() -> &'static SinceTracker {
        Box::leak(Box::new(SinceTracker::default()))
    }

    #[test]
    fn states_are_read_every_look_and_keep_their_start_while_unchanged() {
        let states = Arc::new(parking_lot::Mutex::new(HashMap::from([
            ("b1".to_string(), AgentState::Working),
            ("b2".to_string(), AgentState::Idle),
        ])));
        let read = states.clone();
        let listed = Arc::new(parking_lot::Mutex::new(vec![
            ("AgentX".to_string(), "b1".to_string()),
            ("Camper".to_string(), "b2".to_string()),
            ("Term".to_string(), "b3".to_string()),
        ]));
        let list = listed.clone();
        let since = tracker();
        let source = FleetSource::new(
            Box::new(move || list.lock().clone()),
            Box::new(|_| Some("host")),
            Box::new(move |b| read.lock().get(b).copied()),
            since,
            Box::new(Vec::new),
            "stable".into(),
            String::new(),
        );
        let first = source.observe().agent_status;
        assert_eq!(
            first.keys().collect::<Vec<_>>(),
            ["AgentX", "Camper"],
            "the agent with no state is left out"
        );
        assert_eq!(first["AgentX"].state, AgentState::Working);

        std::thread::sleep(Duration::from_millis(5));
        let second = source.observe().agent_status;
        assert_eq!(second, first, "nothing changed, the starts held");

        states.lock().insert("b1".into(), AgentState::Idle);
        let third = source.observe().agent_status;
        assert_eq!(third["AgentX"].state, AgentState::Idle);
        assert!(third["AgentX"].since_ms > first["AgentX"].since_ms, "a change starts again");
        assert_eq!(third["Camper"], first["Camper"]);

        listed.lock().remove(0);
        source.observe();
        assert_eq!(since.len(), 1, "an agent no longer listed is forgotten");
    }
}
