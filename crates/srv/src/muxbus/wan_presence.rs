// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Publish this install's presence record to the account's relay, so a
//! signed-in phone off the LAN can list it (agentmux-mobile's
//! SPEC_FLEET_HOST_TAGS_AND_CLOUD_HOSTS_2026_10_06 §6.3).
//!
//! The record (`agentmux_common::install_presence`, version 2) is built from
//! the LAN fleet feed's snapshot (agent names, kinds and states, the channel
//! count) plus this install's platform, hostname, channel and version, and
//! signed with the WAN instance key, the same identity `wan_publish`
//! certifies agent keys with. It goes to the account the shared MuxBus
//! sign-in names, with the same token and relay base URL `wan_publish` uses.
//!
//! **When.** The rules live in [`machine`], a pure state machine
//! (`signed_out`, `publishing`, `retrying`, `unsupported`, `rejected`);
//! [`Driver`] runs it. While publishing: on start, on every change of the
//! agents, their kinds or the channel count (debounced [`DEBOUNCE`]), on a
//! change of agent states alone at most every [`STATE_MIN_INTERVAL`]
//! (agentmux-mobile's SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §3.3,
//! so busy agents don't publish on every turn), and every minute otherwise.
//! A failure backs off with jitter; a relay without the route (404) is tried
//! again within minutes, and at once on a sign-in, a network change, a wake
//! from sleep or a new relay version. Not signed in: no request at all. One
//! request at a time, so an older record never lands after a newer one.
//! Neither the token nor the signature is ever logged.
//!
//! `presence.status` reads [`status`]; `presence.publish-now` calls
//! [`publish_now`]; a sign-in or sign-out calls [`sign_in_changed`].

mod machine;

use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use agentmux_common::install_presence::{InstallPresence, PresenceAgent};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::backend::fleet_feed::{FleetFeed, FleetSnapshot};
use crate::backend::rpc_types::PresenceStatusResult;
use crate::backend::storage::store::Store;
use crate::backend::storage::wan_identity::{WanIdentityStore, WanInstance};
use machine::{Action, Answer, Event, Machine, Now};

/// A change is published once the agents, kinds and channel count have been
/// quiet this long.
const DEBOUNCE: Duration = Duration::from_secs(2);

/// The least time between two publishes when only agent states changed.
const STATE_MIN_INTERVAL: Duration = Duration::from_secs(10);

const PUBLISH_TIMEOUT_SECS: u64 = 10;

/// How often the network addresses and the clock are looked at, for a
/// network change or a wake from sleep.
const WATCH_EVERY: Duration = Duration::from_secs(15);

/// A watch tick this much later on the wall clock than it should be means
/// the computer slept in between.
const WAKE_SLACK: Duration = Duration::from_secs(30);

const OUT_OF_BOUNDS: &str = "the record is out of bounds (hostname, channel or version)";

/// The record for `snapshot`, as version `v`. An agent whose kind isn't
/// known is left out: the record requires one, and the feed doesn't guess.
/// An agent whose state isn't known carries none.
pub(crate) fn build_record(
    instance: &WanInstance,
    feed: &FleetFeed,
    snapshot: &FleetSnapshot,
    v: u32,
    published_at_ms: u64,
) -> Option<InstallPresence> {
    let agents = snapshot.agents.iter().filter_map(|name| {
        snapshot.agent_kinds.get(name).map(|kind| PresenceAgent {
            name: name.clone(),
            kind: (*kind).to_string(),
            state: snapshot.agent_status.get(name).map(|s| s.state.as_str().to_string()),
        })
    });
    InstallPresence::sign_version(
        v,
        &instance.private_key,
        feed.hostname(),
        feed.channel(),
        &crate::backend::host_os::sanitize_os(feed.os()).unwrap_or_default(),
        feed.version(),
        snapshot.channels_running,
        agents,
        published_at_ms,
    )
}

/// An HTTP date (`Date`, `Retry-After`) as unix ms.
pub(crate) fn parse_http_date(value: &str) -> Option<u64> {
    let at = chrono::DateTime::parse_from_rfc2822(value.trim()).ok()?;
    u64::try_from(at.timestamp_millis()).ok()
}

/// `Retry-After`: seconds, or a date read against the relay's own `Date`
/// (else this computer's clock).
pub(crate) fn parse_retry_after(value: &str, relay_now_ms: Option<u64>, local_now_ms: u64) -> Option<Duration> {
    if let Ok(secs) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = parse_http_date(value)?;
    Some(Duration::from_millis(at.saturating_sub(relay_now_ms.unwrap_or(local_now_ms))))
}

fn header_date(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    parse_http_date(headers.get(reqwest::header::DATE)?.to_str().ok()?)
}

/// What one relay request came to: the answer, and the relay's clock.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RelayReply {
    pub answer: Answer,
    pub date_ms: Option<u64>,
}

/// `PUT /wan-instances/:instance_id/presence`. A pure HTTP operation, like
/// `wan_publish::publish_record`, so tests can point it at a stub.
pub(crate) async fn put_presence(
    base_url: &str,
    http: &reqwest::Client,
    token: &str,
    record: &InstallPresence,
) -> RelayReply {
    let url = format!(
        "{}/wan-instances/{}/presence",
        base_url.trim_end_matches('/'),
        record.instance_id
    );
    let resp = http
        .put(&url)
        .header("Authorization", format!("Bearer {token}"))
        .timeout(Duration::from_secs(PUBLISH_TIMEOUT_SECS))
        .json(record)
        .send()
        .await;
    let r = match resp {
        Ok(r) => r,
        Err(e) => {
            return RelayReply {
                answer: Answer::Unavailable { reason: "cloud unreachable".into(), detail: e.to_string() },
                date_ms: None,
            }
        }
    };
    let status = r.status().as_u16();
    let date_ms = header_date(r.headers());
    let retry_after = r
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_retry_after(v, date_ms, agentmux_common::time::now_ms_u64()));
    let body = r.text().await.unwrap_or_default();
    let error = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| body.clone());
    let error: String = error.chars().take(200).collect();
    let error = (!error.is_empty()).then_some(error);
    RelayReply { answer: machine::classify(status, error.as_deref(), retry_after), date_ms }
}

/// The relay's unauthenticated `GET /api/health`: its version, and its clock.
pub(crate) async fn relay_health(base_url: &str, http: &reqwest::Client) -> (Option<String>, Option<u64>) {
    #[derive(serde::Deserialize)]
    struct Health {
        version: Option<String>,
    }
    let url = format!("{}/api/health", base_url.trim_end_matches('/'));
    match http.get(&url).timeout(Duration::from_secs(PUBLISH_TIMEOUT_SECS)).send().await {
        Ok(r) => {
            let date_ms = header_date(r.headers());
            let version = if r.status().is_success() {
                r.json::<Health>().await.ok().and_then(|h| h.version).filter(|v| !v.is_empty())
            } else {
                None
            };
            (version, date_ms)
        }
        Err(_) => (None, None),
    }
}

/// What a try needs besides the relay: the sign-in and the signed record.
pub(crate) trait Session: Sync {
    /// The MuxBus access token; `None` when not signed in.
    fn token(&self) -> impl Future<Output = Option<String>> + Send;
    /// The signed record for `snapshot`, or why there is none.
    fn record(&self, snapshot: &FleetSnapshot, v: u32, published_at_ms: u64) -> Result<InstallPresence, String>;
}

struct LiveSession {
    wan: Arc<WanIdentityStore>,
    feed: Arc<FleetFeed>,
    id_store: Arc<Store>,
}

impl Session for LiveSession {
    async fn token(&self) -> Option<String> {
        // The MuxBus credential is registered with the broker once the cloud
        // subscriber runs; without it there is no sign-in to use, and asking
        // the broker to refresh it would only log a warning every minute.
        let scheduler = crate::broker::get_global()?;
        scheduler.state(crate::muxbus::CREDENTIAL_ID)?;
        crate::muxbus::cloud_subscriber::load_valid_token(&self.id_store, &scheduler).await
    }

    fn record(&self, snapshot: &FleetSnapshot, v: u32, published_at_ms: u64) -> Result<InstallPresence, String> {
        let instance = self
            .wan
            .instance_ensure(&crate::backend::reactive::registry::local_host_label())
            .map_err(|e| format!("no instance: {e}"))?;
        build_record(&instance, &self.feed, snapshot, v, published_at_ms).ok_or_else(|| OUT_OF_BOUNDS.to_string())
    }
}

/// One try: the record for `snapshot` as version `v`, or nothing without a
/// sign-in.
pub(crate) async fn attempt<S: Session>(
    session: &S,
    http: &reqwest::Client,
    base_url: &str,
    snapshot: &FleetSnapshot,
    v: u32,
    published_at_ms: u64,
) -> Event {
    let Some(token) = session.token().await else { return Event::NoSignIn };
    let record = match session.record(snapshot, v, published_at_ms) {
        Ok(record) => record,
        Err(why) => return Event::Answer { sent_v: v, answer: Answer::Refused(why), relay_date_ms: None },
    };
    let reply = put_presence(base_url, http, &token, &record).await;
    Event::Answer { sent_v: v, answer: reply.answer, relay_date_ms: reply.date_ms }
}

/// Uniform in `[0, 1)`, for jitter, from the OS's random source.
fn random_unit() -> f64 {
    let bytes = *uuid::Uuid::new_v4().as_bytes();
    let mut word = [0u8; 8];
    // The first six bytes of a v4 UUID are all random.
    word[..6].copy_from_slice(&bytes[..6]);
    u64::from_le_bytes(word) as f64 / (1u64 << 48) as f64
}

/// Runs [`Machine`] against the relay: does what each step says, then waits
/// for the step's wait, a signal ([`Event::SignIn`], [`Event::PublishNow`],
/// [`Event::NetworkChanged`], [`Event::Woke`]) or a change of the snapshot.
/// While publishing, a change is debounced as before; otherwise it only
/// marks the record dirty.
pub(crate) struct Driver<'a, S> {
    pub session: &'a S,
    pub http: &'a reqwest::Client,
    pub base_url: &'a str,
    pub config: &'a machine::Config,
    pub timing: &'a Timing,
    pub status: &'a Mutex<Option<PresenceStatusResult>>,
}

impl<S: Session> Driver<'_, S> {
    pub(crate) async fn run(
        &self,
        mut changes: tokio::sync::watch::Receiver<FleetSnapshot>,
        mut signals: mpsc::UnboundedReceiver<Event>,
        cancel: &CancellationToken,
    ) {
        let started = tokio::time::Instant::now();
        let now = || Now {
            mono_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            wall_ms: agentmux_common::time::now_ms_u64(),
            rand: random_unit(),
        };
        let mut machine = Machine::new(self.config.clone(), self.base_url);
        let mut sent = changes.borrow_and_update().clone();
        let mut sent_at = tokio::time::Instant::now();
        let mut event = Event::Start;
        loop {
            let at = now();
            let step = machine.next(event, at);
            for log in &step.logs {
                if log.warn {
                    tracing::warn!("wan presence: {}", log.text);
                } else {
                    tracing::info!("wan presence: {}", log.text);
                }
            }
            if let (Some(status), Ok(mut slot)) = (machine.status(&at), self.status.lock()) {
                *slot = Some(status);
            }
            event = match step.action {
                Action::Attempt { v } => {
                    sent = changes.borrow_and_update().clone();
                    sent_at = tokio::time::Instant::now();
                    let published_at_ms = machine.stamp(agentmux_common::time::now_ms_u64());
                    attempt(self.session, self.http, self.base_url, &sent, v, published_at_ms).await
                }
                Action::CheckHealth => {
                    let (version, relay_date_ms) = relay_health(self.base_url, self.http).await;
                    Event::Health { version, relay_date_ms }
                }
                Action::Wait => {
                    let follow = machine.follows_changes();
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(step.wait) => Event::Due,
                        Some(signal) = signals.recv() => signal,
                        changed = changes.changed() => {
                            if changed.is_err() {
                                return;
                            }
                            if follow && !wait_to_publish(&mut changes, &sent, sent_at, cancel, self.timing).await {
                                return;
                            }
                            Event::Changed
                        }
                    }
                }
            };
        }
    }
}

static SIGNALS: OnceLock<mpsc::UnboundedSender<Event>> = OnceLock::new();
static STATUS: Mutex<Option<PresenceStatusResult>> = Mutex::new(None);

/// The publisher's status; `None` until it has one (or without a publisher).
pub(crate) fn status() -> Option<PresenceStatusResult> {
    STATUS.lock().ok()?.clone()
}

/// Forget the backoff and try at once. False when no publisher runs.
pub(crate) fn publish_now() -> bool {
    SIGNALS.get().is_some_and(|tx| tx.send(Event::PublishNow).is_ok())
}

/// A sign-in, sign-out or account change in this process: try at once.
pub(crate) fn sign_in_changed() {
    if let Some(tx) = SIGNALS.get() {
        let _ = tx.send(Event::SignIn);
    }
}

/// Start the publisher. No-op without a WAN identity store (nothing to sign
/// with). Stops with `token` (srv shutdown).
pub(crate) fn spawn(feed: Arc<FleetFeed>, id_store: Arc<Store>, token: CancellationToken) {
    let Some(wan) = crate::backend::storage::wan_identity::global() else { return };
    let (tx, rx) = mpsc::unbounded_channel();
    if SIGNALS.set(tx.clone()).is_err() {
        return;
    }
    tokio::spawn(watch_network_and_sleep(tx, token.clone()));
    tokio::spawn(async move {
        let http = reqwest::Client::new();
        let base_url = crate::muxbus::relay::rest_base_url();
        let changes = feed.subscribe();
        let session = LiveSession { wan, feed, id_store };
        let driver = Driver {
            session: &session,
            http: &http,
            base_url: &base_url,
            config: &machine::LIVE,
            timing: &LIVE_TIMING,
            status: &STATUS,
        };
        driver.run(changes, rx, &token).await;
    });
}

/// Whether the computer slept between two watch ticks `tick` apart: the
/// wall clock moved much further than the monotonic clock (which stops
/// during sleep on Linux and macOS), or much further than the tick itself
/// (where the monotonic clock runs on through sleep).
pub(crate) fn woke(tick: Duration, mono_elapsed: Duration, wall_elapsed_ms: i64) -> bool {
    let Ok(wall) = u64::try_from(wall_elapsed_ms) else { return false };
    let wall = Duration::from_millis(wall);
    wall > mono_elapsed + WAKE_SLACK || wall > tick + WAKE_SLACK
}

/// This computer's addresses, loopback aside, sorted; `None` when they
/// can't be read.
fn network_addresses() -> Option<Vec<std::net::IpAddr>> {
    let mut ips: Vec<_> = if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| i.ip())
        .collect();
    ips.sort();
    ips.dedup();
    Some(ips)
}

/// Whether the addresses moved; an unreadable list is no change.
pub(crate) fn network_changed(before: &Option<Vec<std::net::IpAddr>>, after: &Option<Vec<std::net::IpAddr>>) -> bool {
    matches!((before, after), (Some(a), Some(b)) if a != b)
}

/// Every [`WATCH_EVERY`], tell the publisher about a wake from sleep or a
/// change of network addresses. Cheap: a clock read and the interface list.
async fn watch_network_and_sleep(signals: mpsc::UnboundedSender<Event>, cancel: CancellationToken) {
    let mut addresses = network_addresses();
    let mut last = (tokio::time::Instant::now(), agentmux_common::time::now_ms_u64());
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(WATCH_EVERY) => {}
        }
        let here = (tokio::time::Instant::now(), agentmux_common::time::now_ms_u64());
        let slept = woke(WATCH_EVERY, here.0 - last.0, here.1 as i64 - last.1 as i64);
        last = here;
        let current = network_addresses();
        let moved = network_changed(&addresses, &current);
        if current.is_some() {
            addresses = current;
        }
        let event = if slept {
            Some(Event::Woke)
        } else if moved {
            Some(Event::NetworkChanged)
        } else {
            None
        };
        if let Some(event) = event {
            if signals.send(event).is_err() {
                return;
            }
        }
    }
}

/// The publisher's waits, apart so tests can shorten them.
pub(crate) struct Timing {
    debounce: Duration,
    state_min_interval: Duration,
    max_settle: Duration,
}

const LIVE_TIMING: Timing = Timing {
    debounce: DEBOUNCE,
    state_min_interval: STATE_MIN_INTERVAL,
    max_settle: machine::LIVE.interval,
};

/// Whether anything but agent states differs: the agents, their kinds or
/// the channel count.
pub(crate) fn structure_changed(a: &FleetSnapshot, b: &FleetSnapshot) -> bool {
    a.agents != b.agents || a.agent_kinds != b.agent_kinds || a.channels_running != b.channels_running
}

/// After a change of the snapshot, wait until it is time to publish: a
/// change of the agents, kinds or channel count once those have settled
/// ([`settle`]); a change of agent states alone once
/// [`STATE_MIN_INTERVAL`] has passed since `sent_at`. A structural change
/// during that wait switches to settling. `false` on shutdown.
async fn wait_to_publish(
    changes: &mut tokio::sync::watch::Receiver<FleetSnapshot>,
    sent: &FleetSnapshot,
    sent_at: tokio::time::Instant,
    token: &CancellationToken,
    timing: &Timing,
) -> bool {
    let due = sent_at + timing.state_min_interval;
    loop {
        let structural = structure_changed(sent, &changes.borrow_and_update());
        if structural {
            return settle(changes, token, timing).await;
        }
        tokio::select! {
            _ = token.cancelled() => return false,
            _ = tokio::time::sleep_until(due) => return true,
            changed = changes.changed() => {
                if changed.is_err() {
                    return false;
                }
            }
        }
    }
}

/// Wait until the agents, kinds and channel count have been quiet for
/// [`DEBOUNCE`], at most a publish interval in all. A change of states alone
/// does not restart the wait: busy agents would otherwise hold a new agent
/// back for the full interval. `false` on shutdown.
async fn settle(
    changes: &mut tokio::sync::watch::Receiver<FleetSnapshot>,
    token: &CancellationToken,
    timing: &Timing,
) -> bool {
    let deadline = tokio::time::Instant::now() + timing.max_settle;
    let mut reference = changes.borrow_and_update().clone();
    let mut quiet_at = tokio::time::Instant::now() + timing.debounce;
    loop {
        tokio::select! {
            _ = token.cancelled() => return false,
            _ = tokio::time::sleep_until(quiet_at) => return true,
            _ = tokio::time::sleep_until(deadline) => return true,
            changed = changes.changed() => {
                if changed.is_err() {
                    return false;
                }
                let current = changes.borrow_and_update().clone();
                if structure_changed(&reference, &current) {
                    reference = current;
                    quiet_at = tokio::time::Instant::now() + timing.debounce;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
