// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Publish each agent's WAN key to the account's cloud key directory,
//! certified by this install's instance key
//! (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.2, step D1b).
//!
//! A receiver can verify a WAN jekt only against a key it can fetch, and it
//! trusts that key only because this install's instance key signed it — so
//! the directory never has to be trusted, only reachable. The cloud checks
//! the same chain on `PUT` (hygiene); the receiver checks it again.
//!
//! **When.** The spawn path nudges the publisher after it provisions a key
//! ([`nudge`]); a background loop also re-runs every [`RETRY_SECS`] so a key
//! that failed to publish (logged out, cloud down) is retried. A cloud that
//! answers 404 predates the directory (C1 not deployed): retried at most
//! hourly, never hot.
//!
//! **Which token.** The shared account token (`load_valid_token`), not a
//! per-agent M2M token — a per-agent client may be unowned, and the
//! directory is keyed by account.
//!
//! Nothing here ever makes a jekt fail: until a key is confirmed published,
//! the relay simply sends that agent's jekts unsigned (`reactive.rs`'s carry
//! gate, condition 4).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agentmux_common::jekt_sign::WanKeyRecord;

use crate::backend::storage::store::Store;
use crate::backend::storage::wan_identity::{WanAgentKey, WanIdentityStore, WanInstance};

/// Background retry interval while any key is unpublished.
const RETRY_SECS: u64 = 5 * 60;

/// Retry interval after the cloud said it has no directory (HTTP 404).
const NOT_SUPPORTED_RETRY_SECS: u64 = 60 * 60;

const PUBLISH_TIMEOUT_SECS: u64 = 10;

static NUDGE: OnceLock<Arc<tokio::sync::Notify>> = OnceLock::new();

fn nudge_handle() -> Arc<tokio::sync::Notify> {
    NUDGE.get_or_init(|| Arc::new(tokio::sync::Notify::new())).clone()
}

/// Ask the publisher to run a pass soon — called by the spawn path after it
/// provisions an agent's WAN key. Cheap and safe to call with no publisher
/// running.
pub(crate) fn nudge() {
    nudge_handle().notify_one();
}

/// This agent key's directory record, certified by the instance. Channel is
/// the one the agent signs as (`AGENTMUX_CHANNEL`, `local_channel_id`), and
/// `issued_at` is the key's creation time, so the same key always yields the
/// same record.
pub(crate) fn certify(
    instance: &WanInstance,
    agent_id: &str,
    channel: &str,
    key: &WanAgentKey,
) -> Option<WanKeyRecord> {
    WanKeyRecord::certify(
        &instance.private_key,
        agent_id,
        channel,
        &key.public_key_bytes()?,
        &instance.host_hint,
        key.created_at,
    )
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PublishOutcome {
    Published,
    /// HTTP 404: this cloud has no key directory yet.
    NotSupported,
    Failed(String),
}

/// `PUT /agents/:agent_id/wan-key`. A pure HTTP operation, like
/// `relay::relay_inject`, so tests can point it at a stub.
pub(crate) async fn publish_record(
    base_url: &str,
    http: &reqwest::Client,
    token: &str,
    record: &WanKeyRecord,
) -> PublishOutcome {
    let url = format!("{}/agents/{}/wan-key", base_url.trim_end_matches('/'), record.agent_id);
    let resp = http
        .put(&url)
        .header("Authorization", format!("Bearer {token}"))
        .timeout(Duration::from_secs(PUBLISH_TIMEOUT_SECS))
        .json(record)
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => PublishOutcome::Published,
        Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => PublishOutcome::NotSupported,
        Ok(r) => {
            let status = r.status();
            PublishOutcome::Failed(format!("HTTP {status}: {}", r.text().await.unwrap_or_default()))
        }
        Err(e) => PublishOutcome::Failed(format!("unreachable: {e}")),
    }
}

/// Result of one pass over the unpublished keys.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PassResult {
    pub published: usize,
    pub not_supported: bool,
    pub failed: usize,
    /// Keys still unpublished after the pass.
    pub remaining: usize,
}

/// Publish every key `wan` doesn't yet have confirmed. Stops at the first
/// 404 — the whole directory is missing, not one record.
pub(crate) async fn publish_pending(
    wan: &WanIdentityStore,
    instance: &WanInstance,
    channel: &str,
    base_url: &str,
    http: &reqwest::Client,
    token: &str,
) -> PassResult {
    let pending = match wan.agent_keys_unpublished() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "wan publish: could not read wan.db");
            return PassResult { failed: 1, ..Default::default() };
        }
    };
    let mut result = PassResult::default();
    for (agent_id, key) in &pending {
        let Some(record) = certify(instance, agent_id, channel, key) else {
            tracing::warn!(agent = %agent_id, "wan publish: malformed key in wan.db — skipped");
            result.failed += 1;
            continue;
        };
        match publish_record(base_url, http, token, &record).await {
            PublishOutcome::Published => match wan.agent_key_mark_published(agent_id, &key.public_key) {
                Ok(true) => result.published += 1,
                // The key changed under us (agent deleted and recreated
                // mid-PUT); the new key is picked up next pass.
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(agent = %agent_id, error = %e, "wan publish: could not record publication");
                    result.failed += 1;
                }
            },
            PublishOutcome::NotSupported => {
                result.not_supported = true;
                break;
            }
            PublishOutcome::Failed(e) => {
                tracing::warn!(agent = %agent_id, error = %e, "wan publish: directory rejected or unreachable");
                result.failed += 1;
            }
        }
    }
    result.remaining = wan.agent_keys_unpublished().map(|p| p.len()).unwrap_or(pending.len());
    result
}

/// Start the background publisher. No-op without a `wan.db`. Runs a pass on
/// start, on every [`nudge`], and every [`RETRY_SECS`] while keys remain;
/// after a 404 it waits [`NOT_SUPPORTED_RETRY_SECS`] (nudges included, so a
/// burst of spawns can't turn an old cloud into a hot loop).
pub(crate) fn spawn(mstore: Arc<Store>, id_store: Arc<Store>) {
    let Some(wan) = mstore.wan_identity() else { return };
    let notify = nudge_handle();
    tokio::spawn(async move {
        let http = reqwest::Client::new();
        loop {
            let wait = run_pass(&wan, &id_store, &http).await;
            match wait {
                Wait::NotSupported => tokio::time::sleep(Duration::from_secs(NOT_SUPPORTED_RETRY_SECS)).await,
                Wait::Retry => {
                    tokio::select! {
                        _ = notify.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(RETRY_SECS)) => {}
                    }
                }
                Wait::Idle => notify.notified().await,
            }
        }
    });
}

enum Wait {
    /// Everything is published: sleep until a spawn nudges.
    Idle,
    Retry,
    NotSupported,
}

async fn run_pass(wan: &Arc<WanIdentityStore>, id_store: &Arc<Store>, http: &reqwest::Client) -> Wait {
    let instance = match wan.instance_ensure(&crate::backend::reactive::registry::local_host_label()) {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, "wan publish: no instance");
            return Wait::Retry;
        }
    };
    if wan.agent_keys_unpublished().map(|p| p.is_empty()).unwrap_or(false) {
        return Wait::Idle;
    }
    let Some(scheduler) = crate::broker::get_global() else { return Wait::Retry };
    let Some(token) = crate::muxbus::cloud_subscriber::load_valid_token(id_store, &scheduler).await else {
        tracing::debug!("wan publish: not logged in to muxbus — keys stay unpublished");
        return Wait::Retry;
    };
    let result = publish_pending(
        wan,
        &instance,
        &crate::backend::reactive::registry::local_channel_id(),
        &crate::muxbus::relay::rest_base_url(),
        http,
        &token,
    )
    .await;
    if result.published > 0 {
        tracing::info!(published = result.published, instance = %instance.instance_id, "wan publish: agent keys published");
    }
    if result.not_supported {
        tracing::info!("wan publish: cloud has no WAN key directory yet — retrying hourly");
        Wait::NotSupported
    } else if result.remaining > 0 {
        Wait::Retry
    } else {
        Wait::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (agent id from the path, Authorization header, body) per PUT.
    type Seen = Vec<(String, Option<String>, serde_json::Value)>;

    struct Stub {
        seen: Arc<std::sync::Mutex<Seen>>,
        _guard: tokio_util::sync::DropGuard,
        url: String,
    }

    /// A stub directory that answers `status` to every PUT and records it.
    async fn stub_directory(status: axum::http::StatusCode) -> Stub {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let app = axum::Router::new().route(
            "/agents/:agent_id/wan-key",
            axum::routing::put(
                move |axum::extract::Path(agent_id): axum::extract::Path<String>,
                      headers: axum::http::HeaderMap,
                      body: axum::Json<serde_json::Value>| {
                    let sink = sink.clone();
                    async move {
                        let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).map(str::to_string);
                        sink.lock().unwrap().push((agent_id, auth, body.0));
                        (status, "{}")
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

    fn temp_wan() -> (tempfile::TempDir, WanIdentityStore, WanInstance) {
        let dir = tempfile::tempdir().unwrap();
        let wan = WanIdentityStore::open(&dir.path().join("wan.db")).unwrap();
        let instance = wan.instance_ensure("narko").unwrap();
        (dir, wan, instance)
    }

    #[test]
    fn a_certified_record_passes_the_chain_and_is_stable_across_calls() {
        let (_dir, wan, instance) = temp_wan();
        let key = wan.agent_key_ensure("Camper", None).unwrap();
        let record = certify(&instance, "camper", "stable", &key).unwrap();
        assert!(record.check_chain().is_some());
        assert_eq!(record.instance_id, instance.instance_id);
        assert_eq!(record.issued_at, key.created_at);
        assert_eq!(record.host_hint, "narko");
        assert_eq!(certify(&instance, "camper", "stable", &key), Some(record), "same key → same record");
    }

    #[tokio::test]
    async fn a_pass_publishes_every_pending_key_and_marks_it() {
        let stub = stub_directory(axum::http::StatusCode::OK).await;
        let (_dir, wan, instance) = temp_wan();
        wan.agent_key_ensure("camper", None).unwrap();
        wan.agent_key_ensure("lark", None).unwrap();

        let result = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok").await;
        assert_eq!(result, PassResult { published: 2, remaining: 0, ..Default::default() });
        assert!(wan.agent_key_load("camper").unwrap().unwrap().is_published());

        let seen = stub.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        for (agent_id, auth, body) in seen {
            assert_eq!(auth.as_deref(), Some("Bearer tok"));
            let record: WanKeyRecord = serde_json::from_value(body).unwrap();
            assert_eq!(record.agent_id, agent_id, "the path names the record's agent");
            assert!(record.check_chain().is_some(), "what goes on the wire passes the cloud's chain check");
            assert_eq!(record.channel, "stable");
        }

        // Nothing left: a second pass sends nothing.
        let again = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok").await;
        assert_eq!(again, PassResult::default());
        assert_eq!(stub.seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_404_means_no_directory_and_stops_the_pass() {
        let stub = stub_directory(axum::http::StatusCode::NOT_FOUND).await;
        let (_dir, wan, instance) = temp_wan();
        wan.agent_key_ensure("camper", None).unwrap();
        wan.agent_key_ensure("lark", None).unwrap();
        let result = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok").await;
        assert!(result.not_supported);
        assert_eq!(result.remaining, 2);
        assert_eq!(stub.seen.lock().unwrap().len(), 1, "one 404 is enough to know");
    }

    #[tokio::test]
    async fn a_rejection_leaves_the_key_unpublished_for_the_next_pass() {
        let stub = stub_directory(axum::http::StatusCode::BAD_REQUEST).await;
        let (_dir, wan, instance) = temp_wan();
        wan.agent_key_ensure("camper", None).unwrap();
        let result = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok").await;
        assert_eq!(result, PassResult { failed: 1, remaining: 1, ..Default::default() });
        assert!(!wan.agent_key_load("camper").unwrap().unwrap().is_published());
    }

    #[tokio::test]
    async fn an_unreachable_directory_is_a_failure_not_a_hang() {
        let url = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let (_dir, wan, instance) = temp_wan();
        wan.agent_key_ensure("camper", None).unwrap();
        let result = publish_pending(&wan, &instance, "stable", &url, &reqwest::Client::new(), "tok").await;
        assert_eq!(result.failed, 1);
        assert_eq!(result.remaining, 1);
    }

    #[test]
    fn a_publication_marks_only_the_key_it_was_for() {
        let (_dir, wan, _instance) = temp_wan();
        let old = wan.agent_key_ensure("camper", None).unwrap();
        // Agent deleted and recreated while the PUT was in flight.
        wan.agent_keys_delete(&["camper".to_string()]).unwrap();
        let new = wan.agent_key_ensure("camper", None).unwrap();
        assert!(!wan.agent_key_mark_published("camper", &old.public_key).unwrap());
        assert!(!wan.agent_key_load("camper").unwrap().unwrap().is_published());
        assert!(wan.agent_key_mark_published("camper", &new.public_key).unwrap());
        assert!(wan.agent_key_load("camper").unwrap().unwrap().is_published());
    }
}
