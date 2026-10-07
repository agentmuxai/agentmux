// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The names-only fleet feed LAN peers read: `GET /agentmux/fleet` and its
//! Server-Sent Events stream `GET /agentmux/fleet/events`
//! (`docs/specs/SPEC_LAN_FLEET_FEED_2026_10_03.md` §2, which links the
//! mobile spec that owns the wire contract).
//!
//! One task compares this instance's reachable agent names once a second and
//! publishes `(rev, names)` through a `watch`, so the snapshot route and every
//! stream read the same value and no stream can fall behind: a slow reader
//! skips straight to the latest state. Agent names only, the same line
//! `handle_reactive_agent_names` holds for a `lan_key` holder, plus (owner
//! decision 2026-10-06, agentmux-mobile's
//! SPEC_FLEET_HOST_TAGS_AND_CLOUD_HOSTS_2026_10_06 §5.1) where each agent
//! runs (`agent_kinds`), how many channels this machine runs, its platform
//! and this install's id. The cloud presence publisher
//! (`muxbus::wan_presence`) reads the same snapshot.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use futures_util::Stream;
use serde::Serialize;
use tokio::sync::watch;

/// How often the name set is compared.
pub const CHANGE_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Comment line sent on an idle stream, so a client can tell a dead TCP
/// connection (30 s of silence, spec §6) from a quiet fleet.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
/// Concurrent event streams per srv; beyond this a request gets 503, so a LAN
/// peer holding the `lan_key` cannot exhaust the srv.
pub const MAX_STREAMS: usize = 32;
/// The stream's first write: how long an SSE client waits before reconnecting.
const RETRY_PREAMBLE: &[u8] = b"retry: 3000\n\n";
const HEARTBEAT: &[u8] = b": hb\n\n";

/// An agent's kind as the feed reports it: `"host"` or `"container"`
/// (`operator_config_seed::agent_kind`).
pub type AgentKind = &'static str;

/// One state of the fleet. `rev` starts at 1 and goes up by one on every
/// change of any field below; `agents` is sorted case-insensitively and
/// de-duplicated; `agent_kinds` has an entry for each of `agents` whose kind
/// is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetSnapshot {
    pub rev: u64,
    pub agents: Arc<Vec<String>>,
    pub agent_kinds: Arc<BTreeMap<String, AgentKind>>,
    /// This channel plus the other running channels of this machine, 1..=99.
    pub channels_running: u32,
}

/// What one look at the instance found: each reachable agent with its kind,
/// `None` when its block couldn't be read, and the channel count.
#[derive(Debug, Clone, Default)]
pub struct FleetObservation {
    pub agents: Vec<(String, Option<AgentKind>)>,
    pub channels_running: u32,
}

/// The `/agentmux/fleet` body, in the contract's field order.
#[derive(Serialize)]
struct FleetBody<'a> {
    epoch: &'a str,
    rev: u64,
    hostname: &'a str,
    channel: &'a str,
    version: &'a str,
    agents: &'a [String],
    os: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    install_id: Option<&'a str>,
    channels_running: u32,
    agent_kinds: &'a BTreeMap<String, AgentKind>,
}

pub struct FleetFeed {
    /// 16 lowercase hex chars, new on every srv launch: a client that sees a
    /// different epoch knows the instance restarted and takes the snapshot
    /// as-is rather than comparing `rev`s.
    epoch: String,
    hostname: String,
    channel: String,
    version: String,
    os: String,
    /// This install's WAN instance id; `None` without a WAN identity store.
    install_id: Option<String>,
    tx: watch::Sender<FleetSnapshot>,
    streams: Arc<AtomicUsize>,
    heartbeat: Duration,
}

/// [`normalize_agents`] for names alone.
#[cfg(test)]
pub fn normalize_names(names: impl IntoIterator<Item = String>) -> Vec<String> {
    normalize_agents(names.into_iter().map(|n| (n, None))).0
}

/// Sort case-insensitively and drop names that differ only by case
/// (registration keys are lower-cased, so those are one agent). Ties break on
/// the exact string so the result does not depend on input order. Each
/// surviving name keeps its own kind.
fn normalize_agents(
    agents: impl IntoIterator<Item = (String, Option<AgentKind>)>,
) -> (Vec<String>, BTreeMap<String, AgentKind>) {
    let mut keyed: Vec<(String, String, Option<AgentKind>)> = agents
        .into_iter()
        .map(|(n, kind)| (n.to_lowercase(), n, kind))
        .collect();
    keyed.sort();
    keyed.dedup_by(|later, earlier| later.0 == earlier.0);
    let kinds = keyed
        .iter()
        .filter_map(|(_, n, kind)| kind.map(|k| (n.clone(), k)))
        .collect();
    (keyed.into_iter().map(|(_, n, _)| n).collect(), kinds)
}

impl FleetFeed {
    pub fn new(hostname: String, channel: String, version: String) -> Self {
        let epoch = uuid::Uuid::new_v4().simple().to_string()[..16].to_string();
        let (tx, _) = watch::channel(FleetSnapshot {
            rev: 1,
            agents: Arc::new(Vec::new()),
            agent_kinds: Arc::new(BTreeMap::new()),
            channels_running: 1,
        });
        Self {
            epoch,
            hostname,
            channel,
            version,
            os: crate::backend::host_os::local_os(),
            install_id: None,
            tx,
            streams: Arc::new(AtomicUsize::new(0)),
            heartbeat: HEARTBEAT_INTERVAL,
        }
    }

    /// Set the install id the body reports (`fleet_source::install_id`).
    pub fn with_install_id(mut self, install_id: Option<String>) -> Self {
        self.install_id = install_id;
        self
    }

    #[cfg(test)]
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    pub fn channel(&self) -> &str {
        &self.channel
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn os(&self) -> &str {
        &self.os
    }

    pub fn snapshot(&self) -> FleetSnapshot {
        self.tx.borrow().clone()
    }

    /// A receiver that wakes on every change, for the presence publisher.
    pub fn subscribe(&self) -> watch::Receiver<FleetSnapshot> {
        self.tx.subscribe()
    }

    /// `<epoch>:<rev>`: the SSE `id`, and (quoted) the snapshot's `ETag`.
    pub fn event_id(&self, snapshot: &FleetSnapshot) -> String {
        format!("{}:{}", self.epoch, snapshot.rev)
    }

    /// The `/agentmux/fleet` body, on one line.
    pub fn body_json(&self, snapshot: &FleetSnapshot) -> String {
        serde_json::to_string(&FleetBody {
            epoch: &self.epoch,
            rev: snapshot.rev,
            hostname: &self.hostname,
            channel: &self.channel,
            version: &self.version,
            agents: &snapshot.agents,
            os: &self.os,
            install_id: self.install_id.as_deref(),
            channels_running: snapshot.channels_running,
            agent_kinds: &snapshot.agent_kinds,
        })
        .unwrap_or_default()
    }

    /// Record the current fleet; bumps `rev` and wakes every stream only if
    /// it differs from the last one (names, kinds or channel count). Returns
    /// whether it did.
    pub fn observe_fleet(&self, observation: FleetObservation) -> bool {
        let (names, kinds) = normalize_agents(observation.agents);
        let channels_running = observation.channels_running.clamp(1, 99);
        self.tx.send_if_modified(|current| {
            if *current.agents == names
                && *current.agent_kinds == kinds
                && current.channels_running == channels_running
            {
                return false;
            }
            current.rev += 1;
            current.agents = Arc::new(names);
            current.agent_kinds = Arc::new(kinds);
            current.channels_running = channels_running;
            true
        })
    }

    /// [`Self::observe_fleet`] with names only: no kinds, the channel count
    /// unchanged.
    #[cfg(test)]
    pub fn observe(&self, names: impl IntoIterator<Item = String>) -> bool {
        let channels_running = self.snapshot().channels_running;
        self.observe_fleet(FleetObservation {
            agents: names.into_iter().map(|n| (n, None)).collect(),
            channels_running,
        })
    }

    /// Start the change-detection task: `observe` is read now, then every
    /// [`CHANGE_POLL_INTERVAL`] on a blocking thread (it reads the store and
    /// the shared registry) until `token` is cancelled (srv shutdown), the
    /// same stop signal the WAL checkpoint loop watches.
    pub fn spawn_change_detection<F>(
        self: Arc<Self>,
        observe: F,
        token: tokio_util::sync::CancellationToken,
    ) where
        F: Fn() -> FleetObservation + Send + Sync + 'static,
    {
        let observe = Arc::new(observe);
        self.observe_fleet(observe());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(CHANGE_POLL_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        let observe = Arc::clone(&observe);
                        if let Ok(observation) = tokio::task::spawn_blocking(move || observe()).await {
                            self.observe_fleet(observation);
                        }
                    }
                    _ = token.cancelled() => break,
                }
            }
        });
    }

    /// Open one event stream, or `None` when [`MAX_STREAMS`] are already open.
    ///
    /// The slot is held by a guard moved into the stream, so it is released
    /// whenever the stream is dropped: when hyper drops the response body
    /// after the client goes away (noticed at the latest on the next
    /// heartbeat write), or if the response is never sent at all.
    ///
    /// With `last_event_id` equal to the current `<epoch>:<rev>` the client is
    /// already up to date and gets no initial event, only later ones.
    pub fn open_stream(
        self: &Arc<Self>,
        last_event_id: Option<&str>,
    ) -> Option<impl Stream<Item = Result<Bytes, Infallible>> + Send + 'static> {
        let slot = StreamSlot::acquire(&self.streams)?;
        let feed = Arc::clone(self);
        let mut rx = self.tx.subscribe();
        let resume_from = last_event_id.map(str::to_string);
        Some(async_stream::stream! {
            let _slot = slot;
            yield Ok(Bytes::from_static(RETRY_PREAMBLE));
            let current = rx.borrow_and_update().clone();
            if resume_from.as_deref() != Some(feed.event_id(&current).as_str()) {
                yield Ok(feed.event_bytes(&current));
            }
            let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + feed.heartbeat, feed.heartbeat);
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    changed = rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let snapshot = rx.borrow_and_update().clone();
                        yield Ok(feed.event_bytes(&snapshot));
                    }
                    _ = heartbeat.tick() => {
                        yield Ok(Bytes::from_static(HEARTBEAT));
                    }
                }
            }
        })
    }

    /// One `fleet` event. `body_json` is serde output, so it holds no raw
    /// newline that could break the `data:` line.
    fn event_bytes(&self, snapshot: &FleetSnapshot) -> Bytes {
        Bytes::from(format!(
            "event: fleet\nid: {}\ndata: {}\n\n",
            self.event_id(snapshot),
            self.body_json(snapshot)
        ))
    }

    #[cfg(test)]
    pub fn open_streams(&self) -> usize {
        self.streams.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn with_heartbeat(mut self, every: Duration) -> Self {
        self.heartbeat = every;
        self
    }
}

/// A held stream slot; releases it on drop.
struct StreamSlot(Arc<AtomicUsize>);

impl StreamSlot {
    fn acquire(counter: &Arc<AtomicUsize>) -> Option<Self> {
        counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_STREAMS).then_some(n + 1)
            })
            .ok()
            .map(|_| Self(Arc::clone(counter)))
    }
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    fn feed() -> Arc<FleetFeed> {
        Arc::new(FleetFeed::new(
            "narko".into(),
            "stable".into(),
            "0.59.7".into(),
        ))
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    async fn next_chunk(
        stream: &mut (impl Stream<Item = Result<Bytes, Infallible>> + Unpin),
    ) -> String {
        let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("a chunk within 5s")
            .expect("stream open")
            .unwrap();
        String::from_utf8(chunk.to_vec()).unwrap()
    }

    #[test]
    fn names_are_sorted_case_insensitively_and_deduplicated() {
        assert_eq!(
            normalize_names(names(&["clamk", "AgentY", "agenty", "Bravo", "alpha"])),
            names(&["AgentY", "alpha", "Bravo", "clamk"])
        );
        // Independent of input order.
        assert_eq!(
            normalize_names(names(&["agenty", "AgentY"])),
            normalize_names(names(&["AgentY", "agenty"]))
        );
    }

    #[test]
    fn the_epoch_is_sixteen_lowercase_hex_chars_and_new_per_feed() {
        let a = feed();
        assert_eq!(a.epoch().len(), 16);
        assert!(a
            .epoch()
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert_ne!(a.epoch(), feed().epoch());
    }

    #[test]
    fn rev_starts_at_one_and_moves_only_when_the_name_set_changes() {
        let f = feed();
        assert_eq!(f.snapshot().rev, 1);
        assert!(f.observe(names(&["b", "a"])));
        assert_eq!(f.snapshot().rev, 2);
        assert!(!f.observe(names(&["a", "b"])), "same set, other order");
        assert!(
            !f.observe(names(&["a", "a", "b"])),
            "a duplicate is not a change"
        );
        assert_eq!(f.snapshot().rev, 2);
        assert!(f.observe(names(&["a"])));
        assert_eq!(f.snapshot().rev, 3);
        assert_eq!(*f.snapshot().agents, names(&["a"]));
    }

    fn keys_of(body: &str) -> Vec<String> {
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    fn observation(agents: &[(&str, Option<AgentKind>)], channels_running: u32) -> FleetObservation {
        FleetObservation {
            agents: agents.iter().map(|(n, k)| (n.to_string(), *k)).collect(),
            channels_running,
        }
    }

    #[test]
    fn the_body_has_exactly_the_contract_fields() {
        let f = feed();
        f.observe_fleet(observation(&[("Clamk", Some("container")), ("AgentY", Some("host"))], 3));
        let snap = f.snapshot();
        let body = f.body_json(&snap);
        assert!(!body.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let mut want = names(&[
            "epoch",
            "rev",
            "hostname",
            "channel",
            "version",
            "agents",
            "os",
            "channels_running",
            "agent_kinds",
        ]);
        want.sort();
        assert_eq!(keys_of(&body), want, "no install_id without a WAN identity");
        assert_eq!(v["epoch"], f.epoch());
        assert_eq!(v["rev"], 2);
        assert_eq!(v["hostname"], "narko");
        assert_eq!(v["channel"], "stable");
        assert_eq!(v["version"], "0.59.7");
        assert_eq!(v["agents"], serde_json::json!(["AgentY", "Clamk"]));
        assert_eq!(v["os"], crate::backend::host_os::local_os());
        assert_eq!(v["channels_running"], 3);
        assert_eq!(v["agent_kinds"], serde_json::json!({"AgentY": "host", "Clamk": "container"}));
    }

    #[test]
    fn the_install_id_is_reported_when_known() {
        let f = FleetFeed::new("narko".into(), "stable".into(), "0.59.7".into())
            .with_install_id(Some("pqkksqckrolze5wvcs6rqeic4e".into()));
        let body = f.body_json(&f.snapshot());
        assert!(keys_of(&body).contains(&"install_id".to_string()));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["install_id"], "pqkksqckrolze5wvcs6rqeic4e");
    }

    #[test]
    fn a_kind_or_channel_count_change_bumps_rev_like_a_name_change() {
        let f = feed();
        assert!(f.observe_fleet(observation(&[("AgentX", Some("host"))], 1)));
        assert_eq!(f.snapshot().rev, 2);
        assert!(!f.observe_fleet(observation(&[("AgentX", Some("host"))], 1)), "nothing changed");
        assert!(f.observe_fleet(observation(&[("AgentX", Some("container"))], 1)), "kind changed");
        assert_eq!(f.snapshot().rev, 3);
        assert!(f.observe_fleet(observation(&[("AgentX", Some("container"))], 3)), "a channel started");
        assert_eq!(f.snapshot().rev, 4);
        assert!(f.observe_fleet(observation(&[("AgentX", None)], 3)), "kind no longer known");
        assert_eq!(f.snapshot().rev, 5);
        assert!(f.snapshot().agent_kinds.is_empty());
    }

    #[test]
    fn kinds_follow_the_name_that_survives_deduplication_and_counts_are_clamped() {
        let f = feed();
        f.observe_fleet(observation(
            &[("agenty", Some("container")), ("AgentY", Some("host")), ("Clamk", None)],
            500,
        ));
        let snap = f.snapshot();
        assert_eq!(*snap.agents, names(&["AgentY", "Clamk"]));
        assert_eq!(
            *snap.agent_kinds,
            BTreeMap::from([("AgentY".to_string(), "host")]),
            "the unknown kind is left out, not guessed"
        );
        assert_eq!(snap.channels_running, 99);
        f.observe_fleet(observation(&[], 0));
        assert_eq!(f.snapshot().channels_running, 1);
    }

    #[tokio::test]
    async fn a_stream_starts_with_retry_then_the_current_state() {
        let f = feed();
        f.observe(names(&["AgentY"]));
        let mut s = Box::pin(f.open_stream(None).unwrap());
        assert_eq!(next_chunk(&mut s).await, "retry: 3000\n\n");
        let event = next_chunk(&mut s).await;
        let snap = f.snapshot();
        assert_eq!(
            event,
            format!(
                "event: fleet\nid: {}:2\ndata: {}\n\n",
                f.epoch(),
                f.body_json(&snap)
            )
        );
    }

    #[tokio::test]
    async fn a_client_already_up_to_date_gets_no_initial_event_but_does_get_changes() {
        let f = Arc::new(
            FleetFeed::new("narko".into(), "stable".into(), "0.59.7".into())
                .with_heartbeat(Duration::from_secs(3600)),
        );
        f.observe(names(&["AgentY"]));
        let current = f.event_id(&f.snapshot());
        let mut s = Box::pin(f.open_stream(Some(&current)).unwrap());
        assert_eq!(next_chunk(&mut s).await, "retry: 3000\n\n");
        // Nothing else is ready yet.
        assert!(tokio::time::timeout(Duration::from_millis(200), s.next())
            .await
            .is_err());

        f.observe(names(&["AgentY", "Clamk"]));
        let event = next_chunk(&mut s).await;
        assert!(
            event.starts_with(&format!("event: fleet\nid: {}:3\n", f.epoch())),
            "{event}"
        );
        assert!(event.contains(r#""agents":["AgentY","Clamk"]"#), "{event}");
    }

    #[tokio::test]
    async fn a_stale_or_foreign_last_event_id_gets_the_snapshot() {
        let f = feed();
        for last in ["0000000000000000:1", "garbage", ""] {
            let mut s = Box::pin(f.open_stream(Some(last)).unwrap());
            next_chunk(&mut s).await;
            assert!(
                next_chunk(&mut s).await.starts_with("event: fleet\n"),
                "{last}"
            );
        }
    }

    #[tokio::test]
    async fn an_idle_stream_sends_heartbeats() {
        let f = Arc::new(
            FleetFeed::new("h".into(), "c".into(), "v".into())
                .with_heartbeat(Duration::from_millis(50)),
        );
        let mut s = Box::pin(f.open_stream(None).unwrap());
        next_chunk(&mut s).await;
        next_chunk(&mut s).await;
        assert_eq!(next_chunk(&mut s).await, ": hb\n\n");
    }

    #[test]
    fn streams_are_capped_and_a_dropped_stream_frees_its_slot() {
        let f = feed();
        let open: Vec<_> = (0..MAX_STREAMS)
            .map(|_| f.open_stream(None).expect("under the cap"))
            .collect();
        assert_eq!(f.open_streams(), MAX_STREAMS);
        assert!(f.open_stream(None).is_none(), "the 33rd is refused");
        drop(open);
        assert_eq!(f.open_streams(), 0);
        assert!(f.open_stream(None).is_some());
    }

    #[tokio::test]
    async fn change_detection_publishes_and_stops_on_cancel() {
        let f = feed();
        let source = Arc::new(parking_lot::Mutex::new(names(&["a"])));
        let token = tokio_util::sync::CancellationToken::new();
        let read = source.clone();
        f.clone().spawn_change_detection(
            move || FleetObservation {
                agents: read.lock().iter().map(|n| (n.clone(), None)).collect(),
                channels_running: 1,
            },
            token.clone(),
        );
        assert_eq!(
            *f.snapshot().agents,
            names(&["a"]),
            "primed before the first tick"
        );

        let mut rx = f.tx.subscribe();
        *source.lock() = names(&["a", "b"]);
        tokio::time::timeout(Duration::from_secs(5), rx.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*f.snapshot().agents, names(&["a", "b"]));

        token.cancel();
        tokio::time::sleep(CHANGE_POLL_INTERVAL + Duration::from_millis(200)).await;
        let rev = f.snapshot().rev;
        *source.lock() = names(&["c"]);
        tokio::time::sleep(CHANGE_POLL_INTERVAL * 2).await;
        assert_eq!(f.snapshot().rev, rev, "no updates after cancel");
    }
}
