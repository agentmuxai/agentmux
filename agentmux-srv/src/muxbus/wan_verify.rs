// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Verify a same-account agent's WAN signature on a cloud-delivered jekt
//! (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.3–2.6, step D2).
//!
//! Runs in `cloud_subscriber::sync_agent_reactive`, before transcript-request
//! resolution, for each claimed injection. The checks run in the §2.3 order
//! and every "couldn't check" outcome is `None` — only an active failure
//! (envelope moved, certificate chain broken, record for something else,
//! signature bad, replay) is `Some(false)`, the forced-sensitive red flag. A
//! directory that is down or slow gives `None` and the message is delivered
//! anyway: never `/reactive/release`, because nothing re-wakes a released row
//! and it could expire undelivered (§1.2).
//!
//! The HTTP entry point never reaches this: an HTTP caller's self-declared
//! `delivery_tier: "wan"` can't set `wan_verified` (`skip_deserializing`).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentmux_common::jekt_sign::{self, WanCarried, WanCheckFailure, WanKeyRecord};

use crate::backend::reactive::types::{WanInstanceInfo, WanInstanceStatus};
use crate::backend::storage::wan_identity::WanIdentityStore;

/// Directory fetch timeout (§2.3): verification runs per message on the sync
/// path, so a slow directory must not hold delivery up for long.
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

/// How stale an instance's revocation status may be before it is re-checked
/// on the next message from it (§2.3: hourly).
const REVOCATION_REFRESH_SECS: i64 = 60 * 60;

/// Global fetch budget: at most this many directory requests per minute, so
/// a flood of signed messages can't turn this install into a directory
/// hammer. An exhausted budget reads as "unavailable" → `None`.
const FETCHES_PER_MINUTE: u32 = 60;

/// The carried fields of one pending row, as the cloud returned them.
#[derive(Debug, Clone, Default)]
pub(crate) struct CarriedRow {
    pub wan_sig: Option<String>,
    pub wan_msg_id: Option<String>,
    pub wan_ts_secs: Option<i64>,
    pub wan_source_agent: Option<String>,
    pub wan_target_agent: Option<String>,
    pub wan_source_host: Option<String>,
    pub wan_source_channel: Option<String>,
    pub wan_key_fp: Option<String>,
    pub sender_same_account: Option<bool>,
}

/// The §2.3 outcome, ready to copy onto the `InjectionRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct WanVerdict {
    pub verified: Option<bool>,
    pub instance: Option<WanInstanceInfo>,
    pub reason: Option<&'static str>,
    /// What exactly went wrong behind `reason`, when it has a cause worth
    /// reading (the directory's HTTP status, a transport or parse error).
    /// Diagnostics only: logged and audited, never part of the verdict.
    pub detail: Option<String>,
}

impl WanVerdict {
    fn none(reason: &'static str) -> Self {
        Self { verified: None, instance: None, reason: Some(reason), detail: None }
    }

    /// "Couldn't check", with the cause. For the receiver's own setup
    /// failures too, which happen before [`verify`] can run.
    pub(crate) fn unchecked(reason: &'static str, detail: impl Into<String>) -> Self {
        Self { verified: None, instance: None, reason: Some(reason), detail: Some(detail.into()) }
    }

    fn failed(reason: &'static str) -> Self {
        Self { verified: Some(false), instance: None, reason: Some(reason), detail: None }
    }

    fn from_failure(f: WanCheckFailure) -> Self {
        let reason = match f {
            WanCheckFailure::EnvelopeMismatch => "wan_envelope_mismatch",
            WanCheckFailure::Stale => "wan_sig_stale",
            WanCheckFailure::CertInvalid => "wan_cert_invalid",
            WanCheckFailure::RecordMismatch => "wan_record_mismatch",
            WanCheckFailure::BadSignature => "wan_sig_invalid",
        };
        Self { verified: f.verdict(), instance: None, reason: Some(reason), detail: None }
    }
}

/// Where the directory is and how to reach it. Tests point `base_url` at a
/// stub and bring their own budget.
pub(crate) struct Directory<'a> {
    pub base_url: &'a str,
    pub http: &'a reqwest::Client,
    pub token: &'a str,
    pub budget: &'a FetchBudget,
}

enum Fetched<T> {
    Found(T),
    NotFound,
    /// Why, for the verdict's `detail`: the last attempt's cause.
    Unavailable(String),
}

/// A fixed-window request budget ([`FETCHES_PER_MINUTE`]).
pub(crate) struct FetchBudget {
    limit: u32,
    window: Mutex<Option<(Instant, u32)>>,
}

impl FetchBudget {
    pub(crate) const fn new(limit: u32) -> Self {
        Self { limit, window: Mutex::new(None) }
    }

    fn take(&self) -> bool {
        let mut window = self.window.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let (start, used) = window.get_or_insert((now, 0));
        if now.duration_since(*start) >= Duration::from_secs(60) {
            *start = now;
            *used = 0;
        }
        if *used >= self.limit {
            return false;
        }
        *used += 1;
        true
    }
}

/// The one budget every production verification shares.
pub(crate) static GLOBAL_BUDGET: FetchBudget = FetchBudget::new(FETCHES_PER_MINUTE);

/// One GET with the §2.3 retry rule: an unavailable directory gets one
/// immediate retry; a 404 is an answer, not a failure.
async fn get_json<T: serde::de::DeserializeOwned>(dir: &Directory<'_>, url: &str) -> Fetched<T> {
    let mut cause = String::new();
    for _attempt in 0..2 {
        if !dir.budget.take() {
            return Fetched::Unavailable("directory fetch budget exhausted".to_string());
        }
        let resp = dir
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", dir.token))
            .timeout(FETCH_TIMEOUT)
            .send()
            .await;
        match resp {
            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => return Fetched::NotFound,
            Ok(r) if r.status().is_success() => {
                return match r.json::<T>().await {
                    Ok(v) => Fetched::Found(v),
                    // A 200 we can't parse is the cloud's problem; it can't
                    // be a verification failure on the sender's part.
                    Err(e) => Fetched::Unavailable(format!("directory record doesn't parse: {e}")),
                }
            }
            // Still "couldn't check", but never silently: a directory that
            // rejects this install's token turned every WAN jekt
            // `network-claimed` with nothing in the log to say why. The cause
            // is returned, not logged per attempt: the caller logs it once
            // (`cloud_subscriber`'s "wan verify: outcome" at `warn`,
            // `fetch_revoked` below) and the verdict carries it to the audit.
            Ok(r) => cause = format!("directory answered {}", r.status()),
            // `without_url`: the query names the sender's instance and key.
            Err(e) => cause = format!("directory unreachable: {}", e.without_url()),
        }
    }
    Fetched::Unavailable(cause)
}

async fn fetch_record(dir: &Directory<'_>, carried: &WanCarried<'_>) -> Fetched<WanKeyRecord> {
    let mut url = match reqwest::Url::parse(dir.base_url) {
        Ok(u) => u,
        Err(e) => return Fetched::Unavailable(format!("bad directory URL: {e}")),
    };
    match url.path_segments_mut() {
        Ok(mut segs) => {
            segs.pop_if_empty().push("agents").push(&carried.source_agent.to_lowercase()).push("wan-key");
        }
        Err(_) => return Fetched::Unavailable("bad directory URL: cannot be a base".to_string()),
    }
    url.query_pairs_mut()
        .append_pair("instance", &carried.source_host.to_lowercase())
        .append_pair("channel", carried.source_channel)
        .append_pair("fp", carried.key_fp);
    get_json(dir, url.as_str()).await
}

#[derive(serde::Deserialize)]
struct InstanceStatusResp {
    revoked: bool,
}

async fn fetch_revoked(dir: &Directory<'_>, instance_id: &str) -> Option<bool> {
    let url = format!("{}/wan-instances/{}", dir.base_url.trim_end_matches('/'), instance_id);
    match get_json::<InstanceStatusResp>(dir, &url).await {
        Fetched::Found(s) => Some(s.revoked),
        // An older cloud with no such route: not known to be revoked.
        Fetched::NotFound => Some(false),
        // No verdict carries this one, so it is the only place to say it.
        Fetched::Unavailable(cause) => {
            tracing::warn!(instance_id, cause = %cause, "wan verify: revocation check couldn't reach the directory");
            None
        }
    }
}

/// The instance's status (§2.6). This install is approved implicitly. Any
/// other instance is approved only through the host-gated window, which
/// ships disabled until GHSA-6726-q276-g6f6 is fixed, so it stays `new`.
/// Revocation is checked on first sight and at most hourly after.
async fn instance_status(
    wan: &WanIdentityStore,
    own_instance_id: &str,
    record: &WanKeyRecord,
    dir: &Directory<'_>,
    now: i64,
) -> WanInstanceStatus {
    let hint = if jekt_sign::is_valid_wan_host_hint(&record.host_hint) { record.host_hint.as_str() } else { "?" };
    let Ok(mut known) = wan.known_instance_observe(&record.instance_id, hint, now) else {
        return WanInstanceStatus::New;
    };
    let due = known.revocation_checked_at.is_none_or(|t| now.saturating_sub(t) >= REVOCATION_REFRESH_SECS);
    if known.revoked_at.is_none() && due {
        if let Some(revoked) = fetch_revoked(dir, &record.instance_id).await {
            let _ = wan.known_instance_record_revocation_check(&record.instance_id, revoked, now);
            if revoked {
                known.revoked_at = Some(now);
            }
        }
    }
    if known.revoked_at.is_some() {
        WanInstanceStatus::Revoked
    } else if record.instance_id == own_instance_id {
        WanInstanceStatus::Approved
    } else {
        WanInstanceStatus::New
    }
}

/// The §2.3 table, in order.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn verify(
    wan: &WanIdentityStore,
    own_instance_id: &str,
    polled_agent_id: &str,
    row_source_agent: &str,
    row: &CarriedRow,
    message: &str,
    now: i64,
    dir: &Directory<'_>,
) -> WanVerdict {
    // No signature (or no lookup hint): nothing to check.
    let (Some(sig), Some(key_fp)) = (row.wan_sig.as_deref(), row.wan_key_fp.as_deref()) else {
        return WanVerdict::default();
    };
    // Cross-account, legacy, or a cloud that doesn't say.
    if row.sender_same_account != Some(true) {
        return WanVerdict::none("wan_not_same_account");
    }
    let (Some(msg_id), Some(ts_secs), Some(source_agent), Some(target_agent), Some(source_host), Some(source_channel)) = (
        row.wan_msg_id.as_deref(),
        row.wan_ts_secs,
        row.wan_source_agent.as_deref(),
        row.wan_target_agent.as_deref(),
        row.wan_source_host.as_deref(),
        row.wan_source_channel.as_deref(),
    ) else {
        // The cloud stores all eight or none; a partial set is not a
        // signature anyone made.
        return WanVerdict::none("wan_fields_incomplete");
    };
    let carried = WanCarried {
        sig,
        msg_id,
        ts_secs,
        source_agent,
        target_agent,
        source_host,
        source_channel,
        key_fp,
    };
    if let Err(f) = jekt_sign::check_wan_envelope(&carried, row_source_agent, polled_agent_id, now) {
        return WanVerdict::from_failure(f);
    }

    let record = match wan.peer_record_get(source_host, source_agent, source_channel, key_fp) {
        Ok(Some(cached)) => cached,
        _ => match fetch_record(dir, &carried).await {
            Fetched::Found(record) => record,
            Fetched::NotFound => return WanVerdict::none("wan_key_not_found"),
            Fetched::Unavailable(cause) => return WanVerdict::unchecked("wan_key_unavailable", cause),
        },
    };
    if let Err(f) = jekt_sign::verify_wan_against_record(&carried, &record, message) {
        return WanVerdict::from_failure(f);
    }
    // Only a record that passed its chain is ever cached.
    let _ = wan.peer_record_put(&record);

    if wan.seen_sig_contains(&record.instance_id, source_agent, msg_id).unwrap_or(false) {
        return WanVerdict::failed("wan_sig_replay");
    }

    let status = instance_status(wan, own_instance_id, &record, dir, now).await;
    WanVerdict {
        verified: Some(true),
        instance: Some(WanInstanceInfo {
            id: record.instance_id.clone(),
            label: jekt_sign::wan_instance_display_label(&record.host_hint, &record.instance_id),
            status,
        }),
        reason: None,
        detail: None,
    }
}

/// After a verified message was delivered locally: remember it, so the same
/// (instance, agent, msgid) is a replay from now on (§2.5). Not before
/// delivery — the relay's own release-and-redeliver must not look like one.
pub(crate) fn record_delivered(wan: &WanIdentityStore, verdict: &WanVerdict, row: &CarriedRow, now: i64) {
    let (Some(instance), Some(agent), Some(msg_id), Some(ts)) = (
        verdict.instance.as_ref(),
        row.wan_source_agent.as_deref(),
        row.wan_msg_id.as_deref(),
        row.wan_ts_secs,
    ) else {
        return;
    };
    if verdict.verified != Some(true) {
        return;
    }
    if let Err(e) = wan.seen_sig_record(&instance.id, agent, msg_id, ts.saturating_add(jekt_sign::WAN_SIG_MAX_AGE_SECS), now) {
        tracing::warn!(error = %e, "wan verify: could not record a delivered signature — a replay of it would verify");
    }
}

#[cfg(test)]
mod tests;
