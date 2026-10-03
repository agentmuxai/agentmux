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
//! `handle_reactive_agent_names` holds for a `lan_key` holder.

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

/// One state of the name set. `rev` starts at 1 and goes up by one on every
/// change; `agents` is sorted case-insensitively and de-duplicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetSnapshot {
    pub rev: u64,
    pub agents: Arc<Vec<String>>,
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
}

pub struct FleetFeed {
    /// 16 lowercase hex chars, new on every srv launch: a client that sees a
    /// different epoch knows the instance restarted and takes the snapshot
    /// as-is rather than comparing `rev`s.
    epoch: String,
    hostname: String,
    channel: String,
    version: String,
    tx: watch::Sender<FleetSnapshot>,
    streams: Arc<AtomicUsize>,
    heartbeat: Duration,
}

/// Sort case-insensitively and drop names that differ only by case
/// (registration keys are lower-cased, so those are one agent). Ties break on
/// the exact string so the result does not depend on input order.
pub fn normalize_names(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut keyed: Vec<(String, String)> =
        names.into_iter().map(|n| (n.to_lowercase(), n)).collect();
    keyed.sort();
    keyed.dedup_by(|later, earlier| later.0 == earlier.0);
    keyed.into_iter().map(|(_, n)| n).collect()
}

impl FleetFeed {
    pub fn new(hostname: String, channel: String, version: String) -> Self {
        let epoch = uuid::Uuid::new_v4().simple().to_string()[..16].to_string();
        let (tx, _) = watch::channel(FleetSnapshot {
            rev: 1,
            agents: Arc::new(Vec::new()),
        });
        Self {
            epoch,
            hostname,
            channel,
            version,
            tx,
            streams: Arc::new(AtomicUsize::new(0)),
            heartbeat: HEARTBEAT_INTERVAL,
        }
    }

    #[cfg(test)]
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    pub fn snapshot(&self) -> FleetSnapshot {
        self.tx.borrow().clone()
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
        })
        .unwrap_or_default()
    }

    /// Record the current name set; bumps `rev` and wakes every stream only
    /// if it differs from the last one. Returns whether it did.
    pub fn observe(&self, names: impl IntoIterator<Item = String>) -> bool {
        let names = normalize_names(names);
        self.tx.send_if_modified(|current| {
            if *current.agents == names {
                return false;
            }
            current.rev += 1;
            current.agents = Arc::new(names);
            true
        })
    }

    /// Start the change-detection task: `list` is read now, then every
    /// [`CHANGE_POLL_INTERVAL`] until `token` is cancelled (srv shutdown),
    /// the same stop signal the WAL checkpoint loop watches.
    pub fn spawn_change_detection<F>(
        self: Arc<Self>,
        list: F,
        token: tokio_util::sync::CancellationToken,
    ) where
        F: Fn() -> Vec<String> + Send + 'static,
    {
        self.observe(list());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(CHANGE_POLL_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        self.observe(list());
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

    #[test]
    fn the_body_has_exactly_the_contract_fields() {
        let f = feed();
        f.observe(names(&["Clamk", "AgentY"]));
        let snap = f.snapshot();
        let body = f.body_json(&snap);
        assert!(!body.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        let mut want = names(&["epoch", "rev", "hostname", "channel", "version", "agents"]);
        want.sort();
        assert_eq!(keys, want);
        assert_eq!(v["epoch"], f.epoch());
        assert_eq!(v["rev"], 2);
        assert_eq!(v["hostname"], "narko");
        assert_eq!(v["channel"], "stable");
        assert_eq!(v["version"], "0.59.7");
        assert_eq!(v["agents"], serde_json::json!(["AgentY", "Clamk"]));
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
        f.clone()
            .spawn_change_detection(move || read.lock().clone(), token.clone());
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
