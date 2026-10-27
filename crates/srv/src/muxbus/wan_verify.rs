// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Verify a same-account agent's WAN signature on a cloud-delivered jekt
//! (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.3–2.6, step D2).
//!
//! Runs in `cloud_subscriber::delivery::claim_and_deliver`, before transcript-request
//! resolution, for each claimed injection. The checks run in the §2.3 order
//! and every "couldn't check" outcome is `None` — only an active failure
//! (envelope moved, certificate chain broken, record for something else,
//! signature bad, replay) is `Some(false)`, the forced-sensitive red flag. A
//! directory that is down or slow gives `None` and the message is delivered
//! anyway: never `/reactive/release`, because nothing re-wakes a released row
//! and it could expire undelivered (§1.2).
//!
//! **Replaced keys.** An install that replaces an agent's key signs a
//! [`WanKeyRevocation`] for the old one. A message signed by a fingerprint
//! this install holds a revocation of is `Some(false)` (`wan_key_revoked`),
//! whatever its timestamp says, and the key is not looked up again. A revocation
//! is learned from the directory's answer for the key (HTTP 410 with the signed
//! notice) or from the account's feed, read at most every
//! [`KEY_FEED_REFRESH_SECS`] while signed messages arrive; each must verify
//! before it is kept (`wan_identity::revocations`).
//!
//! The HTTP entry point never reaches this: an HTTP caller's self-declared
//! `delivery_tier: "wan"` can't set `wan_verified` (`skip_deserializing`).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentmux_common::jekt_sign::{self, WanCarried, WanCheckFailure, WanKeyRecord};
use agentmux_common::wan_key_revocation::WanKeyRevocation;

use crate::backend::reactive::types::{WanInstanceInfo, WanInstanceStatus};
use crate::backend::storage::wan_identity::WanIdentityStore;

/// Directory fetch timeout (§2.3): verification runs per message on the sync
/// path, so a slow directory must not hold delivery up for long.
const FETCH_TIMEOUT: Duration = Duration::from_secs(2);

/// How stale an instance's revocation status may be before it is re-checked
/// on the next message from it (§2.3: hourly).
const REVOCATION_REFRESH_SECS: i64 = 60 * 60;

/// How long after reading the account's key revocation feed it may be read
/// again, on the next signed message. This bounds how long a receiver that
/// cached a key keeps accepting it after its install replaced it.
const KEY_FEED_REFRESH_SECS: i64 = 5 * 60;

/// How long a cloud without the feed route is left alone.
const KEY_FEED_UNSUPPORTED_SECS: i64 = 60 * 60;

/// Pages of the feed read in one go; the rest waits for the next read.
const KEY_FEED_MAX_PAGES: usize = 4;

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
    /// HTTP 410: the directory no longer serves this. For a key, the body
    /// carries the revocation the install signed.
    Gone(serde_json::Value),
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
            Ok(r) if r.status() == reqwest::StatusCode::GONE => {
                return Fetched::Gone(r.json::<serde_json::Value>().await.unwrap_or_default());
            }
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
    fetch_record_for(dir, carried.source_agent, carried.source_host, carried.source_channel, carried.key_fp).await
}

async fn fetch_record_for(dir: &Directory<'_>, agent: &str, instance: &str, channel: &str, key_fp: &str) -> Fetched<WanKeyRecord> {
    let mut url = match reqwest::Url::parse(dir.base_url) {
        Ok(u) => u,
        Err(e) => return Fetched::Unavailable(format!("bad directory URL: {e}")),
    };
    match url.path_segments_mut() {
        Ok(mut segs) => {
            segs.pop_if_empty().push("agents").push(&agent.to_lowercase()).push("wan-key");
        }
        Err(_) => return Fetched::Unavailable("bad directory URL: cannot be a base".to_string()),
    }
    url.query_pairs_mut()
        .append_pair("instance", &instance.to_lowercase())
        .append_pair("channel", channel)
        .append_pair("fp", key_fp);
    get_json(dir, url.as_str()).await
}

#[derive(serde::Deserialize)]
struct InstanceStatusResp {
    revoked: bool,
    /// The signed record behind `revoked` (§2.2). The directory returns it
    /// whenever it says an instance is revoked.
    #[serde(default)]
    revocation: Option<jekt_sign::WanRevocation>,
}

/// Whether `instance_id` is revoked, `None` when that can't be told.
///
/// A revocation is sticky and turns every later message from the install into
/// a sensitive one, so a bare `revoked: true` is not enough: the directory has
/// to hand over the revocation itself, and it has to be the one the instance
/// signed with its own key for this id. An answer that says revoked without it
/// is not acted on.
async fn fetch_revoked(dir: &Directory<'_>, instance_id: &str) -> Option<bool> {
    let url = format!("{}/wan-instances/{}", dir.base_url.trim_end_matches('/'), instance_id);
    match get_json::<InstanceStatusResp>(dir, &url).await {
        Fetched::Found(s) if !s.revoked => Some(false),
        Fetched::Found(s) => match s.revocation {
            Some(r) if r.old_instance_id == instance_id && r.verify() => Some(true),
            _ => {
                tracing::warn!(instance_id, "wan verify: the directory says an instance is revoked without a revocation it signed: not acted on");
                None
            }
        },
        // An older cloud with no such route: not known to be revoked.
        Fetched::NotFound => Some(false),
        // Not an answer this route gives.
        Fetched::Gone(_) => None,
        // No verdict carries this one, so it is the only place to say it.
        Fetched::Unavailable(cause) => {
            tracing::warn!(instance_id, cause = %cause, "wan verify: revocation check couldn't reach the directory");
            None
        }
    }
}

/// The revocation a 410 carries, if it is one the install signed for exactly
/// this key. Anything else is not acted on: the directory can say a key is
/// gone, but only the install can retire it.
fn gone_revocation(body: &serde_json::Value, instance: &str, agent: &str, channel: &str, key_fp: &str) -> Option<WanKeyRevocation> {
    let revocation: WanKeyRevocation = serde_json::from_value(body.get("revocation")?.clone()).ok()?;
    (revocation.names(instance, agent, channel, key_fp) && revocation.verify()).then_some(revocation)
}

#[derive(serde::Deserialize)]
struct KeyRevocationFeed {
    #[serde(default)]
    revocations: Vec<WanKeyRevocation>,
    #[serde(default)]
    next_since: Option<i64>,
    #[serde(default)]
    more: bool,
}

/// Read the account's key revocations the directory has received since the
/// last read, keep those that verify, and drop the cached records they name.
/// At most once per [`KEY_FEED_REFRESH_SECS`] per directory; a failed read is
/// not retried sooner, so a down directory costs one request per interval.
async fn refresh_key_revocations(wan: &WanIdentityStore, dir: &Directory<'_>, now: i64) {
    let directory = crate::muxbus::wan_publish::directory_id(dir.base_url, &crate::muxbus::pkce::token_sub(dir.token));
    let Ok((mut since, checked_at)) = wan.key_revocation_feed_state(&directory) else { return };
    if now.saturating_sub(checked_at) < KEY_FEED_REFRESH_SECS {
        return;
    }
    let _ = wan.key_revocation_feed_note(&directory, now, None);
    for _ in 0..KEY_FEED_MAX_PAGES {
        let url = format!("{}/wan-key-revocations?since={since}", dir.base_url.trim_end_matches('/'));
        match get_json::<KeyRevocationFeed>(dir, &url).await {
            Fetched::Found(feed) => {
                let mut all_kept = true;
                for revocation in &feed.revocations {
                    match wan.revoked_key_put(revocation, now) {
                        Ok(true) => {}
                        Ok(false) => tracing::warn!("wan verify: the directory's key revocation feed held one that does not verify: ignored"),
                        Err(e) => {
                            tracing::warn!(error = %e, "wan verify: could not keep a key revocation");
                            all_kept = false;
                        }
                    }
                }
                // A revocation that could not be stored must be read again:
                // the cursor stays where it was, and the next read retries.
                if !all_kept {
                    return;
                }
                let next = feed.next_since.unwrap_or(since).max(since);
                let _ = wan.key_revocation_feed_note(&directory, now, Some(next));
                if !feed.more || next == since {
                    return;
                }
                since = next;
            }
            // A cloud without the feed: ask again in an hour.
            Fetched::NotFound => {
                let _ = wan.key_revocation_feed_note(&directory, now + KEY_FEED_UNSUPPORTED_SECS - KEY_FEED_REFRESH_SECS, None);
                return;
            }
            Fetched::Gone(_) => return,
            Fetched::Unavailable(cause) => {
                tracing::warn!(cause = %cause, "wan verify: the key revocation feed couldn't be read");
                return;
            }
        }
    }
}

/// Whether this install holds a verified revocation of the key. `Err` when
/// that can't be told.
fn key_is_revoked(wan: &WanIdentityStore, instance: &str, agent: &str, channel: &str, key_fp: &str) -> Result<bool, String> {
    wan.revoked_key_exists(instance, agent, channel, key_fp).map_err(|e| e.to_string())
}

/// What a 410 for this key means: refused if the install signed its
/// retirement (kept, so the next message needn't ask), otherwise a directory
/// answer that can't be acted on.
fn gone_verdict(wan: &WanIdentityStore, body: &serde_json::Value, instance: &str, agent: &str, channel: &str, key_fp: &str, now: i64) -> WanVerdict {
    match gone_revocation(body, instance, agent, channel, key_fp) {
        Some(revocation) => {
            let _ = wan.revoked_key_put(&revocation, now);
            WanVerdict::failed("wan_key_revoked")
        }
        None => WanVerdict::unchecked("wan_key_unavailable", "directory answered 410 without a revocation the install signed for this key"),
    }
}

#[derive(serde::Deserialize)]
struct PresenceList {
    #[serde(default)]
    instances: Vec<PresenceRow>,
}

#[derive(serde::Deserialize)]
struct PresenceRow {
    instance_id: String,
    /// A tombstone: the install said goodbye and is listed only as offline.
    #[serde(default)]
    gone: bool,
}

/// Whether the account's presence list shows `instance_id` as a live install.
/// The directory lists only installs that published a record signed by their
/// own key and that are not revoked, so this is what separates an install of
/// the account from a key someone merely published. `None` when the list
/// couldn't be read.
async fn fetch_listed(dir: &Directory<'_>, instance_id: &str) -> Option<bool> {
    let url = format!("{}/wan-instances", dir.base_url.trim_end_matches('/'));
    match get_json::<PresenceList>(dir, &url).await {
        Fetched::Found(list) => Some(list.instances.iter().any(|row| !row.gone && row.instance_id == instance_id)),
        Fetched::NotFound => Some(false),
        Fetched::Gone(_) => None,
        Fetched::Unavailable(cause) => {
            tracing::warn!(instance_id, cause = %cause, "wan verify: the presence list couldn't be read");
            None
        }
    }
}

/// What the pin said about the install that signed a message.
enum PinCheck {
    /// Pinned to this install, or not pinnable yet.
    Allowed,
    /// The name is pinned to another install.
    Mismatch(String),
    /// The pin store couldn't be read.
    Unavailable(String),
}

/// `(agent, channel)` is pinned to the first install the account's presence
/// list shows speaking for it. A message from another install for a pinned
/// name fails; a name with no pin is pinned only once its install is listed,
/// so a key published by something that is not an install of the account
/// pins nothing. This install's own agents never go through here.
async fn check_agent_instance_pin(
    wan: &WanIdentityStore,
    record: &WanKeyRecord,
    agent: &str,
    channel: &str,
    dir: &Directory<'_>,
    now: i64,
) -> PinCheck {
    let hint = if jekt_sign::is_valid_wan_host_hint(&record.host_hint) { record.host_hint.as_str() } else { "?" };
    let pinned = match wan.agent_instance_pin_get(agent, channel) {
        Ok(p) => p,
        Err(e) => return PinCheck::Unavailable(e.to_string()),
    };
    let pin = match pinned {
        Some(pin) => pin,
        None => {
            if fetch_listed(dir, &record.instance_id).await != Some(true) {
                return PinCheck::Allowed;
            }
            match wan.agent_instance_pin_get_or_set(agent, channel, &record.instance_id, hint, now) {
                Ok(pin) => pin,
                Err(e) => return PinCheck::Unavailable(e.to_string()),
            }
        }
    };
    if pin.instance_id == record.instance_id {
        return PinCheck::Allowed;
    }
    let _ = wan.agent_instance_pin_note_conflict(agent, channel, &record.instance_id, hint, now);
    PinCheck::Mismatch(format!(
        "pinned to {}, this came from {}",
        jekt_sign::wan_instance_display_label(&pin.host_hint, &pin.instance_id),
        jekt_sign::wan_instance_display_label(hint, &record.instance_id),
    ))
}

/// The instance's status (§2.6). This install is approved implicitly. Any
/// other instance is approved only through the host-gated window, which
/// ships disabled, so it stays `new`.
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

    // A key its install retired is refused before anything else is read for it.
    refresh_key_revocations(wan, dir, now).await;
    match key_is_revoked(wan, source_host, source_agent, source_channel, key_fp) {
        Ok(false) => {}
        Ok(true) => return WanVerdict::failed("wan_key_revoked"),
        Err(cause) => return WanVerdict::unchecked("wan_revocation_unavailable", cause),
    }

    let record = match wan.peer_record_get(source_host, source_agent, source_channel, key_fp) {
        Ok(Some(cached)) => cached,
        _ => match fetch_record(dir, &carried).await {
            Fetched::Found(record) => record,
            Fetched::NotFound => return WanVerdict::none("wan_key_not_found"),
            Fetched::Gone(body) => return gone_verdict(wan, &body, source_host, source_agent, source_channel, key_fp, now),
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

    if record.instance_id != own_instance_id {
        match check_agent_instance_pin(wan, &record, source_agent, source_channel, dir, now).await {
            PinCheck::Allowed => {}
            PinCheck::Mismatch(detail) => {
                return WanVerdict { detail: Some(detail), ..WanVerdict::failed("wan_instance_pin_mismatch") };
            }
            PinCheck::Unavailable(cause) => return WanVerdict::unchecked("wan_pin_unavailable", cause),
        }
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

/// A *receiver's* record, for a sender sealing files to it
/// (`server::jekt_cloud`, file transfer spec §7): `agent`'s key on install
/// `instance`, channel `channel`, fingerprint `key_fp`, as the agent's lease
/// named them. Only a record whose chain holds is returned or cached, and
/// never one of a revoked install. `Err` says why not, to follow the agent's
/// name.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn recipient_record(
    wan: &WanIdentityStore,
    own_instance_id: &str,
    dir: &Directory<'_>,
    agent: &str,
    instance: &str,
    channel: &str,
    key_fp: &str,
    now: i64,
) -> Result<WanKeyRecord, String> {
    const RETIRED: &str = "has a key its install retired; wait for the agent's new one";
    refresh_key_revocations(wan, dir, now).await;
    if key_is_revoked(wan, instance, agent, channel, key_fp).map_err(|e| format!("couldn't be checked for a retired key ({e})"))? {
        return Err(RETIRED.to_string());
    }
    let record = match wan.peer_record_get(instance, agent, channel, key_fp) {
        Ok(Some(cached)) => cached,
        _ => match fetch_record_for(dir, agent, instance, channel, key_fp).await {
            Fetched::Found(record) => record,
            Fetched::NotFound => return Err("has no key in the account's key directory yet; try again in a minute".to_string()),
            Fetched::Gone(body) => {
                return Err(match gone_revocation(&body, instance, agent, channel, key_fp) {
                    Some(revocation) => {
                        let _ = wan.revoked_key_put(&revocation, now);
                        RETIRED.to_string()
                    }
                    None => "couldn't be looked up in the account's key directory (it answered 410 without a revocation that checks out)".to_string(),
                });
            }
            Fetched::Unavailable(cause) => return Err(format!("couldn't be looked up in the account's key directory ({cause})")),
        },
    };
    if record.check_chain().is_none() {
        return Err("has a key record whose certificate doesn't check out".to_string());
    }
    let _ = wan.peer_record_put(&record);
    if instance_status(wan, own_instance_id, &record, dir, now).await == WanInstanceStatus::Revoked {
        return Err("runs on an install its owner revoked".to_string());
    }
    Ok(record)
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
mod key_revocation_tests;
#[cfg(test)]
mod tests;
