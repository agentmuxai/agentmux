// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The account's other installs, read back from the relay for the Swarm's cloud tier
//! (`docs/specs/SPEC_SWARM_CLOUD_TIER_2026_11_14.md`): the presence records `wan_presence` publishes, listed by
//! the relay for the signed-in account.
//!
//! A record is display data. Each one is verified before it is used (the install's own signature, its id against
//! its key, the bounds), so the relay's list is trusted for availability and not for correctness; nothing acts on
//! one. The read is on demand, behind a 30-second cache: nothing runs while nobody asks. Signed out, a relay
//! without the route, a refusal or a network error each leave no cloud tier and no error in an answer; after a
//! network error the last good list is kept for ten minutes, ageing as time goes.

use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::install_presence::InstallPresence;

/// A read this old is read again on the next ask.
const FRESH_MS: u64 = 30_000;
/// How long the last good list is kept while the relay cannot be read.
const KEEP_LAST_GOOD_MS: u64 = 10 * 60_000;
/// How long an ask waits for a read before it answers from the cache.
const READ_BUDGET: Duration = Duration::from_millis(1_500);
/// The relay answers at most this many records, and a body past this size is not read.
const MAX_RECORDS: usize = 500;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// One verified record of another install, with when the relay heard from it on this machine's clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudInstall {
    pub record: InstallPresence,
    pub seen_at_ms: u64,
}

/// How the last read went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudState {
    /// Signed out, or the relay has no such list.
    Off,
    Ok,
    /// The last read failed: the list is the one kept, or empty.
    Unavailable,
}

impl CloudState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ok => "ok",
            Self::Unavailable => "unavailable",
        }
    }
}

/// What the Swarm reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudView {
    pub installs: Vec<CloudInstall>,
    pub state: CloudState,
}

impl CloudView {
    /// The state the list gives the agent called `name` on any install, case-insensitively: `working`,
    /// `waiting`, `idle`, `stopped` or `error`; `None` when it is in no record, or its record says none.
    pub fn agent_state(&self, name: &str) -> Option<&str> {
        let agents = self.installs.iter().flat_map(|c| c.record.agents.iter());
        agents.filter(|a| a.name.eq_ignore_ascii_case(name)).find_map(|a| a.state.as_deref())
    }
}

/// What one read of the relay came to.
#[derive(Debug)]
pub enum Fetched {
    /// No sign-in, or the relay refused it.
    SignedOut,
    /// The relay does not have the list (an older relay).
    NoRoute,
    /// Unreachable, a server error or an answer that is not the list.
    Failed,
    /// The records as the relay sent them, and the relay's clock.
    Records { raw: Vec<serde_json::Value>, relay_now_ms: Option<u64> },
}

/// Where the list is read from.
pub trait Source: Send + Sync + 'static {
    fn fetch(&self) -> impl Future<Output = Fetched> + Send;
}

/// The records that verify, with their ages moved onto this machine's clock. A record the relay heard from
/// `age` ago was seen `age` before `local_now_ms`, whatever this clock says about the relay's: so a wrong local
/// clock does not make a live machine stale. A goodbye is left out.
pub fn verified(raw: Vec<serde_json::Value>, relay_now_ms: Option<u64>, local_now_ms: u64) -> Vec<CloudInstall> {
    let mut dropped = 0usize;
    let mut out: Vec<CloudInstall> = Vec::new();
    for value in raw.into_iter().take(MAX_RECORDS) {
        let received = value.get("received_at_ms").and_then(serde_json::Value::as_u64);
        let record = serde_json::from_value::<InstallPresence>(value).ok().filter(InstallPresence::verify);
        let (Some(record), Some(received)) = (record, received) else {
            dropped += 1;
            continue;
        };
        if record.gone {
            continue;
        }
        let seen_at_ms = match relay_now_ms {
            Some(relay_now) => local_now_ms.saturating_sub(relay_now.saturating_sub(received)),
            None => received,
        };
        match out.iter_mut().find(|c| c.record.instance_id == record.instance_id) {
            Some(known) if known.seen_at_ms >= seen_at_ms => {}
            Some(known) => *known = CloudInstall { record, seen_at_ms },
            None => out.push(CloudInstall { record, seen_at_ms }),
        }
    }
    if dropped > 0 {
        tracing::warn!(dropped, "cloud installs: records that did not verify were left out");
    }
    out
}

struct Cache {
    view: CloudView,
    /// When the cache was last filled, on this machine's clock; `None` before the first read and after
    /// [`Reader::invalidate`].
    read_at_ms: Option<u64>,
    /// When the list last came from a read that worked.
    good_at_ms: Option<u64>,
}

/// The cache and the one read at a time.
pub struct Reader<S> {
    source: S,
    cache: Mutex<Cache>,
    reading: tokio::sync::Mutex<()>,
}

impl<S: Source> Reader<S> {
    pub fn new(source: S) -> Self {
        let view = CloudView { installs: Vec::new(), state: CloudState::Off };
        Self { source, cache: Mutex::new(Cache { view, read_at_ms: None, good_at_ms: None }), reading: tokio::sync::Mutex::new(()) }
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The list as of `now_ms`: the cache when it is fresh; else one read, which an ask does not wait on for
    /// more than [`READ_BUDGET`] (it answers from the old cache then), and which a second ask does not repeat.
    pub async fn view(&self, now_ms: u64) -> CloudView {
        let fresh = |cache: &Cache| cache.read_at_ms.is_some_and(|at| now_ms.saturating_sub(at) < FRESH_MS);
        if fresh(&self.cache()) {
            return self.cache().view.clone();
        }
        if let Ok(_one_at_a_time) = self.reading.try_lock() {
            if let Ok(fetched) = tokio::time::timeout(READ_BUDGET, self.source.fetch()).await {
                self.absorb(fetched, now_ms);
            }
        }
        self.cache().view.clone()
    }

    /// The next ask reads again: a sign-in changed.
    pub fn invalidate(&self) {
        self.cache().read_at_ms = None;
    }

    fn absorb(&self, fetched: Fetched, now_ms: u64) {
        let mut cache = self.cache();
        cache.read_at_ms = Some(now_ms);
        cache.view = match fetched {
            Fetched::SignedOut | Fetched::NoRoute => CloudView { installs: Vec::new(), state: CloudState::Off },
            Fetched::Records { raw, relay_now_ms } => {
                cache.good_at_ms = Some(now_ms);
                CloudView { installs: verified(raw, relay_now_ms, now_ms), state: CloudState::Ok }
            }
            Fetched::Failed => {
                let kept = cache.good_at_ms.is_some_and(|at| now_ms.saturating_sub(at) < KEEP_LAST_GOOD_MS);
                let installs = if kept { std::mem::take(&mut cache.view.installs) } else { Vec::new() };
                CloudView { installs, state: CloudState::Unavailable }
            }
        };
    }
}

/// The relay's list, with the MuxBus sign-in's token.
pub struct RelaySource {
    id_store: Arc<crate::backend::storage::store::Store>,
    http: reqwest::Client,
}

impl Source for RelaySource {
    async fn fetch(&self) -> Fetched {
        let Some(scheduler) = crate::broker::get_global() else { return Fetched::SignedOut };
        if scheduler.state(crate::muxbus::CREDENTIAL_ID).is_none() {
            return Fetched::SignedOut;
        }
        let Some(token) = crate::muxbus::cloud_subscriber::load_valid_token(&self.id_store, &scheduler).await else {
            return Fetched::SignedOut;
        };
        let url = format!("{}/wan-instances", crate::muxbus::relay::rest_base_url().trim_end_matches('/'));
        let Ok(reply) = self.http.get(url).header("Authorization", format!("Bearer {token}")).timeout(Duration::from_secs(5)).send().await else {
            return Fetched::Failed;
        };
        match reply.status().as_u16() {
            404 => return Fetched::NoRoute,
            401 | 403 => return Fetched::SignedOut,
            200..=299 => {}
            _ => return Fetched::Failed,
        }
        let relay_now_ms = reply.headers().get(reqwest::header::DATE).and_then(|v| v.to_str().ok()).and_then(super::wan_presence::parse_http_date);
        if reply.content_length().is_some_and(|n| n > MAX_BODY_BYTES as u64) {
            return Fetched::Failed;
        }
        let Ok(body) = reply.bytes().await else { return Fetched::Failed };
        if body.len() > MAX_BODY_BYTES {
            return Fetched::Failed;
        }
        match serde_json::from_slice::<serde_json::Value>(&body).ok().and_then(|v| match v.get("instances") {
            Some(serde_json::Value::Array(list)) => Some(list.clone()),
            _ => None,
        }) {
            Some(raw) => Fetched::Records { raw, relay_now_ms },
            None => Fetched::Failed,
        }
    }
}

static READER: OnceLock<Reader<RelaySource>> = OnceLock::new();

/// Start reading with this srv's sign-in. The first call wins.
pub fn install(id_store: Arc<crate::backend::storage::store::Store>, http: reqwest::Client) {
    let _ = READER.set(Reader::new(RelaySource { id_store, http }));
}

/// The account's other installs now, or `None` before [`install`] (a build with no sign-in to read with).
pub async fn view() -> Option<CloudView> {
    Some(READER.get()?.view(agentmux_common::time::now_ms_u64()).await)
}

/// A sign-in or sign-out happened: the next ask reads again.
pub fn invalidate() {
    if let Some(reader) = READER.get() {
        reader.invalidate();
    }
}

#[cfg(test)]
mod tests;
