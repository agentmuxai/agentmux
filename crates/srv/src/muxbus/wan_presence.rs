// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Publish this install's presence record to the account's relay, so a
//! signed-in phone off the LAN can list it (agentmux-mobile's
//! SPEC_FLEET_HOST_TAGS_AND_CLOUD_HOSTS_2026_10_06 §6.3).
//!
//! The record (`agentmux_common::install_presence`) is built from the LAN
//! fleet feed's snapshot (agent names and kinds, the channel count) plus
//! this install's platform, hostname, channel and version, and signed with
//! the WAN instance key, the same identity `wan_publish` certifies agent
//! keys with. It goes to the account the shared MuxBus sign-in names, with
//! the same token and relay base URL `wan_publish` uses.
//!
//! **When.** On start, on every change of the snapshot (debounced
//! [`DEBOUNCE`]), and every [`INTERVAL_SECS`] otherwise. Not signed in: no
//! request at all. A relay that answers 404 predates the route: retried
//! hourly, changes included. Any other failure: retried at the next tick.
//! One request at a time, so an older record never lands after a newer one.
//! Neither the token nor the signature is ever logged.

use std::sync::Arc;
use std::time::Duration;

use agentmux_common::install_presence::{InstallPresence, PresenceAgent};

use crate::backend::fleet_feed::{FleetFeed, FleetSnapshot};
use crate::backend::storage::store::Store;
use crate::backend::storage::wan_identity::{WanIdentityStore, WanInstance};

/// How often an unchanged record is published again. The phone shows an
/// install as live for three of these.
const INTERVAL_SECS: u64 = 60;

/// Retry interval after the relay said it has no presence route (HTTP 404).
const NOT_SUPPORTED_RETRY_SECS: u64 = 60 * 60;

/// A change is published once the snapshot has been quiet this long.
const DEBOUNCE: Duration = Duration::from_secs(2);

const PUBLISH_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PresenceOutcome {
    Published,
    /// No MuxBus sign-in: nothing was sent.
    SignedOut,
    /// HTTP 404: this relay has no presence route yet.
    NotSupported,
    Failed(String),
}

/// The record for `snapshot`. An agent whose kind isn't known is left out:
/// the record requires one, and the feed doesn't guess.
pub(crate) fn build_record(
    instance: &WanInstance,
    feed: &FleetFeed,
    snapshot: &FleetSnapshot,
    published_at_ms: u64,
) -> Option<InstallPresence> {
    let agents = snapshot.agents.iter().filter_map(|name| {
        snapshot.agent_kinds.get(name).map(|kind| PresenceAgent {
            name: name.clone(),
            kind: (*kind).to_string(),
        })
    });
    InstallPresence::sign(
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

/// `PUT /wan-instances/:instance_id/presence`. A pure HTTP operation, like
/// `wan_publish::publish_record`, so tests can point it at a stub.
pub(crate) async fn put_presence(
    base_url: &str,
    http: &reqwest::Client,
    token: &str,
    record: &InstallPresence,
) -> PresenceOutcome {
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
    match resp {
        Ok(r) if r.status().is_success() => PresenceOutcome::Published,
        Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => PresenceOutcome::NotSupported,
        Ok(r) => {
            let status = r.status();
            let body: String = r.text().await.unwrap_or_default().chars().take(200).collect();
            PresenceOutcome::Failed(format!("HTTP {status}: {body}"))
        }
        Err(e) => PresenceOutcome::Failed(format!("unreachable: {e}")),
    }
}

/// Build and send the record for `snapshot`, or nothing without a `token`.
pub(crate) async fn publish_snapshot(
    instance: &WanInstance,
    feed: &FleetFeed,
    snapshot: &FleetSnapshot,
    token: Option<&str>,
    base_url: &str,
    http: &reqwest::Client,
    now_ms: u64,
) -> PresenceOutcome {
    let Some(token) = token else { return PresenceOutcome::SignedOut };
    let Some(record) = build_record(instance, feed, snapshot, now_ms) else {
        return PresenceOutcome::Failed("the record is out of bounds (hostname, channel or version)".into());
    };
    put_presence(base_url, http, token, &record).await
}

/// How long to wait after `outcome`, and whether a snapshot change may cut
/// the wait short (not after a 404: an old relay is retried hourly only).
pub(crate) fn wait_after(outcome: &PresenceOutcome) -> (Duration, bool) {
    match outcome {
        PresenceOutcome::NotSupported => (Duration::from_secs(NOT_SUPPORTED_RETRY_SECS), false),
        _ => (Duration::from_secs(INTERVAL_SECS), true),
    }
}

/// Start the publisher. No-op without a WAN identity store (nothing to sign
/// with). Stops with `token` (srv shutdown).
pub(crate) fn spawn(feed: Arc<FleetFeed>, id_store: Arc<Store>, token: tokio_util::sync::CancellationToken) {
    let Some(wan) = crate::backend::storage::wan_identity::global() else { return };
    tokio::spawn(async move {
        let http = reqwest::Client::new();
        let mut changes = feed.subscribe();
        let mut last_logged: Option<String> = None;
        loop {
            changes.borrow_and_update();
            let outcome = run_pass(&wan, &feed, &id_store, &http).await;
            log_outcome(&outcome, &mut last_logged);
            let (wait, follow_changes) = wait_after(&outcome);
            tokio::select! {
                _ = token.cancelled() => return,
                _ = tokio::time::sleep(wait) => {}
                changed = changes.changed(), if follow_changes => {
                    if changed.is_err() || !settle(&mut changes, &token).await {
                        return;
                    }
                }
            }
        }
    });
}

/// Wait until the snapshot has been quiet for [`DEBOUNCE`], at most
/// [`INTERVAL_SECS`] in all. `false` on shutdown.
async fn settle(changes: &mut tokio::sync::watch::Receiver<FleetSnapshot>, token: &tokio_util::sync::CancellationToken) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(INTERVAL_SECS);
    loop {
        tokio::select! {
            _ = token.cancelled() => return false,
            _ = tokio::time::sleep(DEBOUNCE) => return true,
            _ = tokio::time::sleep_until(deadline) => return true,
            changed = changes.changed() => {
                if changed.is_err() {
                    return false;
                }
            }
        }
    }
}

/// Log a change of outcome once, not every minute.
fn log_outcome(outcome: &PresenceOutcome, last_logged: &mut Option<String>) {
    let key = match outcome {
        PresenceOutcome::Failed(e) => format!("failed:{e}"),
        other => format!("{other:?}"),
    };
    if last_logged.as_deref() == Some(key.as_str()) {
        return;
    }
    match outcome {
        PresenceOutcome::Published => tracing::info!("wan presence: published"),
        PresenceOutcome::SignedOut => tracing::debug!("wan presence: not signed in to muxbus, nothing published"),
        PresenceOutcome::NotSupported => {
            tracing::info!("wan presence: relay has no presence route yet, retrying hourly")
        }
        PresenceOutcome::Failed(e) => tracing::warn!(error = %e, "wan presence: not published, retrying next tick"),
    }
    *last_logged = Some(key);
}

async fn run_pass(
    wan: &Arc<WanIdentityStore>,
    feed: &FleetFeed,
    id_store: &Arc<Store>,
    http: &reqwest::Client,
) -> PresenceOutcome {
    let instance = match wan.instance_ensure(&crate::backend::reactive::registry::local_host_label()) {
        Ok(i) => i,
        Err(e) => return PresenceOutcome::Failed(format!("no instance: {e}")),
    };
    // The MuxBus credential is registered with the broker once the cloud
    // subscriber runs; without it there is no sign-in to use, and asking the
    // broker to refresh it would only log a warning every minute.
    let Some(scheduler) = crate::broker::get_global() else { return PresenceOutcome::SignedOut };
    if scheduler.state(crate::muxbus::CREDENTIAL_ID).is_none() {
        return PresenceOutcome::SignedOut;
    }
    let token = crate::muxbus::cloud_subscriber::load_valid_token(id_store, &scheduler).await;
    publish_snapshot(
        &instance,
        feed,
        &feed.snapshot(),
        token.as_deref(),
        &crate::muxbus::relay::rest_base_url(),
        http,
        agentmux_common::time::now_ms_u64(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fleet_feed::FleetObservation;

    /// (instance id from the path, Authorization header, body) per PUT.
    type Seen = Vec<(String, Option<String>, serde_json::Value)>;

    struct Stub {
        seen: Arc<std::sync::Mutex<Seen>>,
        _guard: tokio_util::sync::DropGuard,
        url: String,
    }

    async fn stub_relay(status: axum::http::StatusCode) -> Stub {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let app = axum::Router::new().route(
            "/wan-instances/:instance_id/presence",
            axum::routing::put(
                move |axum::extract::Path(instance_id): axum::extract::Path<String>,
                      headers: axum::http::HeaderMap,
                      body: axum::Json<serde_json::Value>| {
                    let sink = sink.clone();
                    async move {
                        let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).map(str::to_string);
                        sink.lock().unwrap().push((instance_id, auth, body.0));
                        (status, "{\"stored\":true}")
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let child = token.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).with_graceful_shutdown(async move { child.cancelled().await }).await;
        });
        Stub { seen, _guard: token.drop_guard(), url: format!("http://{addr}") }
    }

    fn instance() -> (tempfile::TempDir, WanInstance) {
        let dir = tempfile::tempdir().unwrap();
        let wan = WanIdentityStore::open(&dir.path().join("wan.db")).unwrap();
        let instance = wan.instance_ensure("narko").unwrap();
        (dir, instance)
    }

    fn feed() -> FleetFeed {
        let f = FleetFeed::new("narko".into(), "stable".into(), "0.59.11".into());
        f.observe_fleet(FleetObservation {
            agents: vec![
                ("Camper".into(), Some("host")),
                ("AgentX".into(), Some("container")),
                ("Ghost".into(), None),
            ],
            channels_running: 3,
        });
        f
    }

    #[test]
    fn the_record_carries_the_snapshot_and_verifies() {
        let (_dir, instance) = instance();
        let f = feed();
        let record = build_record(&instance, &f, &f.snapshot(), 1_791_352_493_388).unwrap();
        assert!(record.verify());
        assert_eq!(record.instance_id, instance.instance_id);
        assert_eq!(record.hostname, "narko");
        assert_eq!(record.channel, "stable");
        assert_eq!(record.version, "0.59.11");
        assert_eq!(record.os, crate::backend::host_os::local_os());
        assert_eq!(record.channels_running, 3);
        assert_eq!(
            record.agents,
            vec![
                PresenceAgent { name: "AgentX".into(), kind: "container".into() },
                PresenceAgent { name: "Camper".into(), kind: "host".into() },
            ],
            "sorted, and the agent of unknown kind left out"
        );
    }

    #[tokio::test]
    async fn a_publish_puts_a_verifiable_record_under_its_instance_id() {
        let stub = stub_relay(axum::http::StatusCode::OK).await;
        let (_dir, instance) = instance();
        let f = feed();
        let outcome =
            publish_snapshot(&instance, &f, &f.snapshot(), Some("tok"), &stub.url, &reqwest::Client::new(), 42).await;
        assert_eq!(outcome, PresenceOutcome::Published);
        let seen = stub.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        let (path_id, auth, body) = &seen[0];
        assert_eq!(auth.as_deref(), Some("Bearer tok"));
        let record: InstallPresence = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(&record.instance_id, path_id, "the path names the record's instance");
        assert_eq!(record.published_at_ms, 42);
        assert!(record.verify(), "what goes on the wire passes the relay's check");
    }

    #[tokio::test]
    async fn signed_out_sends_nothing() {
        let stub = stub_relay(axum::http::StatusCode::OK).await;
        let (_dir, instance) = instance();
        let f = feed();
        let outcome =
            publish_snapshot(&instance, &f, &f.snapshot(), None, &stub.url, &reqwest::Client::new(), 42).await;
        assert_eq!(outcome, PresenceOutcome::SignedOut);
        assert!(stub.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_404_backs_off_hourly_and_ignores_changes() {
        let stub = stub_relay(axum::http::StatusCode::NOT_FOUND).await;
        let (_dir, instance) = instance();
        let f = feed();
        let outcome =
            publish_snapshot(&instance, &f, &f.snapshot(), Some("tok"), &stub.url, &reqwest::Client::new(), 42).await;
        assert_eq!(outcome, PresenceOutcome::NotSupported);
        assert_eq!(wait_after(&outcome), (Duration::from_secs(3600), false));
    }

    #[tokio::test]
    async fn any_other_failure_retries_at_the_next_tick() {
        let stub = stub_relay(axum::http::StatusCode::GONE).await;
        let (_dir, instance) = instance();
        let f = feed();
        let outcome =
            publish_snapshot(&instance, &f, &f.snapshot(), Some("tok"), &stub.url, &reqwest::Client::new(), 42).await;
        assert!(matches!(outcome, PresenceOutcome::Failed(ref e) if e.starts_with("HTTP 410")), "{outcome:?}");
        assert_eq!(wait_after(&outcome), (Duration::from_secs(60), true));
        assert_eq!(wait_after(&PresenceOutcome::Published), (Duration::from_secs(60), true));
        assert_eq!(wait_after(&PresenceOutcome::SignedOut), (Duration::from_secs(60), true));
    }

    #[tokio::test]
    async fn an_unreachable_relay_is_a_failure_not_a_hang() {
        let url = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let (_dir, instance) = instance();
        let f = feed();
        let outcome = publish_snapshot(&instance, &f, &f.snapshot(), Some("tok"), &url, &reqwest::Client::new(), 42).await;
        assert!(matches!(outcome, PresenceOutcome::Failed(_)));
    }
}
