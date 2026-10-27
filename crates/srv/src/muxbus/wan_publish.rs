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
//! that failed to publish (logged out, cloud down) is retried, and every
//! [`IDLE_RECHECK_SECS`] when idle, because a sign-in in another channel can
//! change the directory without nudging this process. A cloud that
//! answers 404 predates the directory (C1 not deployed): retried at most
//! hourly, never hot.
//!
//! **Which token.** The shared account token (`load_valid_token`), not a
//! per-agent M2M token — a per-agent client may be unowned, and the
//! directory is keyed by account.
//!
//! **Retired keys.** When this install replaces an agent's key, `wan.db` writes
//! down a retirement for each directory that held the old one
//! (`wan_identity::revocations`). The same pass signs a
//! [`WanKeyRevocation`] for each with the instance key and puts it there, and the
//! retirement goes once the directory has it. The revocation is also kept
//! locally at once, so this install refuses the old key even while the
//! directory is out of reach.
//!
//! Nothing here ever makes a jekt fail: until a key is confirmed published,
//! the relay simply sends that agent's jekts unsigned
//! (`relay::wan_sign_at_origin`, condition 2).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agentmux_common::jekt_sign::WanKeyRecord;
use agentmux_common::wan_key_revocation::WanKeyRevocation;

use crate::backend::storage::store::Store;
use crate::backend::storage::wan_identity::{WanAgentKey, WanIdentityStore, WanInstance};

/// Background retry interval while any key is unpublished.
const RETRY_SECS: u64 = 5 * 60;

/// Retry interval after the cloud said it has no directory (HTTP 404).
const NOT_SUPPORTED_RETRY_SECS: u64 = 60 * 60;

/// How often an idle publisher looks again. A sign-in in another channel
/// (the MuxBus login is shared) changes the directory this one should
/// publish to, but only nudges its own process (Codex P2 on #3994), so an
/// idle publisher can't wait on nudges alone.
const IDLE_RECHECK_SECS: u64 = 5 * 60;

const PUBLISH_TIMEOUT_SECS: u64 = 10;

/// Which directory a publication is for: the relay's REST base plus the
/// account (the token's `sub`). The directory is keyed by account, and a
/// relay move or a sign-in to another account is a directory that has never
/// seen this install's keys — so `wan.db` records publications per this id,
/// and a key counts as published only to the directory it went to.
pub(crate) fn directory_id(base_url: &str, account_sub: &str) -> String {
    format!("{}#{}", base_url.trim_end_matches('/'), account_sub)
}

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

/// `PUT /agents/:agent_id/wan-key/revocation`: tell the directory a key was
/// retired. The same outcomes as [`publish_record`]; a 404 here means the
/// cloud has no revocation route (it may still have the key directory).
pub(crate) async fn publish_revocation(
    base_url: &str,
    http: &reqwest::Client,
    token: &str,
    revocation: &WanKeyRevocation,
) -> PublishOutcome {
    let url = format!("{}/agents/{}/wan-key/revocation", base_url.trim_end_matches('/'), revocation.agent_id);
    let resp = http
        .put(&url)
        .header("Authorization", format!("Bearer {token}"))
        .timeout(Duration::from_secs(PUBLISH_TIMEOUT_SECS))
        .json(revocation)
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

/// Result of one pass over the unpublished keys and the retired ones.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PassResult {
    pub published: usize,
    /// Retired keys the directory now has the revocation of.
    pub revoked: usize,
    pub not_supported: bool,
    /// The cloud has the key directory but no revocation route: retirements
    /// wait, and are tried again hourly.
    pub revocations_not_supported: bool,
    pub failed: usize,
    /// Keys still unpublished, and retirements still owed, after the pass.
    pub remaining: usize,
}

/// Tell `directory` about every key this install retired that it held. Each
/// revocation is kept locally before it is sent. Stops at the first 404.
#[allow(clippy::too_many_arguments)]
async fn publish_retirements(
    wan: &WanIdentityStore,
    instance: &WanInstance,
    channel: &str,
    base_url: &str,
    http: &reqwest::Client,
    token: &str,
    directory: &str,
    result: &mut PassResult,
) {
    let pending = match wan.retirements_pending(directory) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "wan publish: could not read retired keys from wan.db");
            result.failed += 1;
            return;
        }
    };
    for retirement in pending {
        let Some(revocation) =
            WanKeyRevocation::sign(&instance.private_key, &retirement.agent_id, channel, &retirement.key_fp, retirement.retired_at)
        else {
            tracing::warn!(agent = %retirement.agent_id, "wan publish: could not sign a key revocation — skipped");
            result.failed += 1;
            continue;
        };
        if let Err(e) = wan.revoked_key_put(&revocation, agentmux_common::time::now_secs()) {
            tracing::warn!(agent = %retirement.agent_id, error = %e, "wan publish: could not keep a key revocation locally");
        }
        match publish_revocation(base_url, http, token, &revocation).await {
            PublishOutcome::Published => match wan.retirement_done(&retirement.agent_id, &retirement.key_fp, directory) {
                Ok(()) => result.revoked += 1,
                Err(e) => {
                    tracing::warn!(agent = %retirement.agent_id, error = %e, "wan publish: could not record a revocation as sent");
                    result.failed += 1;
                }
            },
            PublishOutcome::NotSupported => {
                result.revocations_not_supported = true;
                return;
            }
            PublishOutcome::Failed(e) => {
                tracing::warn!(agent = %retirement.agent_id, error = %e, "wan publish: directory rejected or unreachable for a key revocation");
                result.failed += 1;
            }
        }
    }
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
    directory: &str,
) -> PassResult {
    let pending = match wan.agent_keys_unpublished(directory) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "wan publish: could not read wan.db");
            return PassResult { failed: 1, ..Default::default() };
        }
    };
    let mut result = PassResult::default();
    // Retired keys first: a key that should no longer be trusted is the more urgent news.
    publish_retirements(wan, instance, channel, base_url, http, token, directory, &mut result).await;
    for (agent_id, key) in &pending {
        let Some(record) = certify(instance, agent_id, channel, key) else {
            tracing::warn!(agent = %agent_id, "wan publish: malformed key in wan.db — skipped");
            result.failed += 1;
            continue;
        };
        match publish_record(base_url, http, token, &record).await {
            // Marked for `directory`, the one this PUT went to, even if a
            // sign-in to another account has landed since: the new directory
            // still sees the key as pending.
            PublishOutcome::Published => match wan.agent_key_mark_published(agent_id, &key.public_key, directory) {
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
    result.remaining = wan.agent_keys_unpublished(directory).map(|p| p.len()).unwrap_or(pending.len());
    if !result.revocations_not_supported {
        result.remaining += wan.retirements_pending(directory).map(|p| p.len()).unwrap_or(0);
    }
    result
}

/// Start the background publisher. No-op without a `wan.db`. Runs a pass on
/// start, on every [`nudge`], every [`RETRY_SECS`] while keys remain, and every
/// [`IDLE_RECHECK_SECS`] when none do;
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
                Wait::Idle => {
                    tokio::select! {
                        _ = notify.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(IDLE_RECHECK_SECS)) => {}
                    }
                }
            }
        }
    });
}

enum Wait {
    /// Everything is published to the current directory: sleep until a
    /// nudge, or [`IDLE_RECHECK_SECS`] in case the account changed elsewhere.
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
    let Some(scheduler) = crate::broker::get_global() else { return Wait::Retry };
    let Some(token) = crate::muxbus::cloud_subscriber::load_valid_token(id_store, &scheduler).await else {
        tracing::debug!("wan publish: not logged in to muxbus — keys stay unpublished");
        return Wait::Retry;
    };
    let account = crate::muxbus::pkce::token_sub(&token);
    if account.is_empty() {
        tracing::warn!("wan publish: the account token names no account — keys stay unpublished");
        return Wait::Retry;
    }
    let base_url = crate::muxbus::relay::rest_base_url();
    let directory = directory_id(&base_url, &account);
    let nothing_owed = wan.agent_keys_unpublished(&directory).map(|p| p.is_empty()).unwrap_or(false)
        && wan.retirements_pending(&directory).map(|p| p.is_empty()).unwrap_or(false);
    if nothing_owed {
        return Wait::Idle;
    }
    let result = publish_pending(
        wan,
        &instance,
        &crate::backend::reactive::registry::local_channel_id(),
        &base_url,
        http,
        &token,
        &directory,
    )
    .await;
    if result.published > 0 {
        tracing::info!(published = result.published, instance = %instance.instance_id, "wan publish: agent keys published");
    }
    if result.revoked > 0 {
        tracing::info!(revoked = result.revoked, instance = %instance.instance_id, "wan publish: retired agent keys revoked");
    }
    if result.not_supported {
        tracing::info!("wan publish: cloud has no WAN key directory yet — retrying hourly");
        Wait::NotSupported
    } else if result.remaining > 0 {
        Wait::Retry
    } else if result.revocations_not_supported {
        tracing::info!("wan publish: cloud has no key revocation route yet — retrying hourly");
        Wait::NotSupported
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
        /// The same, for `PUT /agents/:agent_id/wan-key/revocation`.
        revoked: Arc<std::sync::Mutex<Seen>>,
        _guard: tokio_util::sync::DropGuard,
        url: String,
    }

    /// A stub directory that answers `status` to every PUT and records it.
    async fn stub_directory(status: axum::http::StatusCode) -> Stub {
        stub_with_revocations(status, axum::http::StatusCode::OK).await
    }

    /// As [`stub_directory`], answering `revocation_status` to a key revocation.
    async fn stub_with_revocations(status: axum::http::StatusCode, revocation_status: axum::http::StatusCode) -> Stub {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let revoked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (sink, revoked_sink) = (seen.clone(), revoked.clone());
        let app = axum::Router::new()
            .route(
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
            )
            .route(
                "/agents/:agent_id/wan-key/revocation",
                axum::routing::put(
                    move |axum::extract::Path(agent_id): axum::extract::Path<String>,
                          headers: axum::http::HeaderMap,
                          body: axum::Json<serde_json::Value>| {
                        let sink = revoked_sink.clone();
                        async move {
                            let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).map(str::to_string);
                            sink.lock().unwrap().push((agent_id, auth, body.0));
                            (revocation_status, "{}")
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
        Stub { seen, revoked, _guard: token.drop_guard(), url: format!("http://{addr}") }
    }

    const DIR: &str = "https://relay.test#acct";

    fn published(wan: &WanIdentityStore, agent_id: &str, directory: &str) -> bool {
        let key = wan.agent_key_load(agent_id).unwrap().unwrap();
        wan.agent_key_is_published_to(agent_id, &key, directory).unwrap()
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

        let result = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok", DIR).await;
        assert_eq!(result, PassResult { published: 2, remaining: 0, ..Default::default() });
        assert!(published(&wan, "camper", DIR));

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
        let again = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok", DIR).await;
        assert_eq!(again, PassResult::default());
        assert_eq!(stub.seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_404_means_no_directory_and_stops_the_pass() {
        let stub = stub_directory(axum::http::StatusCode::NOT_FOUND).await;
        let (_dir, wan, instance) = temp_wan();
        wan.agent_key_ensure("camper", None).unwrap();
        wan.agent_key_ensure("lark", None).unwrap();
        let result = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok", DIR).await;
        assert!(result.not_supported);
        assert_eq!(result.remaining, 2);
        assert_eq!(stub.seen.lock().unwrap().len(), 1, "one 404 is enough to know");
    }

    #[tokio::test]
    async fn a_rejection_leaves_the_key_unpublished_for_the_next_pass() {
        let stub = stub_directory(axum::http::StatusCode::BAD_REQUEST).await;
        let (_dir, wan, instance) = temp_wan();
        wan.agent_key_ensure("camper", None).unwrap();
        let result = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok", DIR).await;
        assert_eq!(result, PassResult { failed: 1, remaining: 1, ..Default::default() });
        assert!(!published(&wan, "camper", DIR));
    }

    #[tokio::test]
    async fn an_unreachable_directory_is_a_failure_not_a_hang() {
        let url = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let (_dir, wan, instance) = temp_wan();
        wan.agent_key_ensure("camper", None).unwrap();
        let result = publish_pending(&wan, &instance, "stable", &url, &reqwest::Client::new(), "tok", DIR).await;
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
        assert!(!wan.agent_key_mark_published("camper", &old.public_key, DIR).unwrap());
        assert!(!published(&wan, "camper", DIR));
        assert!(wan.agent_key_mark_published("camper", &new.public_key, DIR).unwrap());
        assert!(published(&wan, "camper", DIR));
    }

    #[tokio::test]
    async fn a_sign_in_to_another_account_publishes_every_key_again() {
        // The relay moved (or the user signed in to another account): the
        // new directory has never seen these keys. Before directory-bound
        // marks, the old marks made this pass a no-op and every jekt from
        // here arrived unverified.
        let stub = stub_directory(axum::http::StatusCode::OK).await;
        let (_dir, wan, instance) = temp_wan();
        wan.agent_key_ensure("camper", None).unwrap();
        let old = directory_id(&stub.url, "old-account");
        let new = directory_id(&stub.url, "new-account");
        let first = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok", &old).await;
        assert_eq!(first.published, 1);

        let second = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok2", &new).await;
        assert_eq!(second, PassResult { published: 1, remaining: 0, ..Default::default() });
        assert!(published(&wan, "camper", &new));
        assert!(published(&wan, "camper", &old), "the old directory's record stays true");
        assert_eq!(stub.seen.lock().unwrap().len(), 2);
    }

    /// camper's key is published to `DIR`, then replaced by a new one.
    fn replaced_after_publishing(wan: &WanIdentityStore) -> String {
        let old = wan.agent_key_ensure("camper", None).unwrap();
        assert!(wan.agent_key_mark_published("camper", &old.public_key, DIR).unwrap());
        let fp = agentmux_common::jekt_sign::wan_key_fingerprint(&old.public_key_bytes().unwrap());
        wan.agent_keys_delete(&["camper".to_string()]).unwrap();
        wan.agent_key_ensure("camper", None).unwrap();
        fp
    }

    #[tokio::test]
    async fn a_replaced_key_is_revoked_at_the_directory_that_held_it() {
        let stub = stub_directory(axum::http::StatusCode::OK).await;
        let (_dir, wan, instance) = temp_wan();
        let old_fp = replaced_after_publishing(&wan);

        let result = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok", DIR).await;
        assert_eq!(result, PassResult { published: 1, revoked: 1, remaining: 0, ..Default::default() });

        let sent = stub.revoked.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        let (agent_id, auth, body) = sent[0].clone();
        assert_eq!((agent_id.as_str(), auth.as_deref()), ("camper", Some("Bearer tok")));
        let revocation: WanKeyRevocation = serde_json::from_value(body).unwrap();
        assert!(revocation.verify(), "what goes on the wire passes the cloud's check");
        assert!(revocation.names(&instance.instance_id, "camper", "stable", &old_fp));
        assert!(wan.retirements_pending(DIR).unwrap().is_empty());
        assert!(wan.revoked_key_exists(&instance.instance_id, "camper", "stable", &old_fp).unwrap(), "refused here too");
        // The new key is not the revoked one.
        let current = wan.agent_key_load("camper").unwrap().unwrap();
        let current_fp = agentmux_common::jekt_sign::wan_key_fingerprint(&current.public_key_bytes().unwrap());
        assert!(!wan.revoked_key_exists(&instance.instance_id, "camper", "stable", &current_fp).unwrap());

        // Nothing is owed any more: a second pass sends nothing.
        let again = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok", DIR).await;
        assert_eq!(again, PassResult::default());
        assert_eq!(stub.revoked.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_revocation_that_failed_is_sent_again_unchanged() {
        let down = stub_with_revocations(axum::http::StatusCode::OK, axum::http::StatusCode::SERVICE_UNAVAILABLE).await;
        let up = stub_directory(axum::http::StatusCode::OK).await;
        let (_dir, wan, instance) = temp_wan();
        replaced_after_publishing(&wan);

        let first = publish_pending(&wan, &instance, "stable", &down.url, &reqwest::Client::new(), "tok", DIR).await;
        assert_eq!(first.revoked, 0);
        assert_eq!(first.failed, 1);
        assert_eq!(first.remaining, 1, "still owed");
        assert_eq!(wan.retirements_pending(DIR).unwrap().len(), 1);

        let second = publish_pending(&wan, &instance, "stable", &up.url, &reqwest::Client::new(), "tok", DIR).await;
        assert_eq!(second.revoked, 1);
        assert_eq!(second.remaining, 0);
        assert_eq!(down.revoked.lock().unwrap()[0].2, up.revoked.lock().unwrap()[0].2, "the same signed record, not a new one");
    }

    #[tokio::test]
    async fn this_install_refuses_the_retired_key_even_when_the_directory_cannot_be_reached() {
        let url = {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let (_dir, wan, instance) = temp_wan();
        let old_fp = replaced_after_publishing(&wan);
        let result = publish_pending(&wan, &instance, "stable", &url, &reqwest::Client::new(), "tok", DIR).await;
        assert!(result.failed >= 1);
        assert_eq!(wan.retirements_pending(DIR).unwrap().len(), 1, "still owed to the directory");
        assert!(wan.revoked_key_exists(&instance.instance_id, "camper", "stable", &old_fp).unwrap());
    }

    #[tokio::test]
    async fn a_cloud_without_the_revocation_route_still_gets_the_keys_and_is_asked_again_later() {
        let stub = stub_with_revocations(axum::http::StatusCode::OK, axum::http::StatusCode::NOT_FOUND).await;
        let (_dir, wan, instance) = temp_wan();
        replaced_after_publishing(&wan);
        let result = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok", DIR).await;
        assert_eq!(result.published, 1, "the new key is published all the same");
        assert!(result.revocations_not_supported);
        assert!(!result.not_supported);
        assert_eq!(result.remaining, 0, "not counted, or the pass would run hot");
        assert_eq!(wan.retirements_pending(DIR).unwrap().len(), 1, "kept for when the route exists");
    }

    #[tokio::test]
    async fn another_directory_is_not_told_about_a_key_it_never_held() {
        let stub = stub_directory(axum::http::StatusCode::OK).await;
        let (_dir, wan, instance) = temp_wan();
        replaced_after_publishing(&wan);
        let other = directory_id(&stub.url, "other-account");
        let result = publish_pending(&wan, &instance, "stable", &stub.url, &reqwest::Client::new(), "tok2", &other).await;
        assert_eq!(result.revoked, 0);
        assert!(stub.revoked.lock().unwrap().is_empty());
        assert_eq!(wan.retirements_pending(DIR).unwrap().len(), 1, "DIR is still owed");
    }

    #[test]
    fn a_directory_is_the_relay_plus_the_account() {
        assert_eq!(directory_id("https://relay.example/", "sub-1"), "https://relay.example#sub-1");
        assert_ne!(directory_id("https://relay.example", "sub-1"), directory_id("https://relay.example", "sub-2"));
        assert_ne!(directory_id("https://a.example", "sub-1"), directory_id("https://b.example", "sub-1"));
    }
}
