// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One live instance per agent across the WAN — the client side
//! (`docs/specs/SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md` Phase 5, §4.5).
//!
//! The muxbus relay holds one lease per agent (agentmux-cloud
//! `agent-lease-store.ts`). This instance claims it for every agent it polls
//! jekts for, renews it while the agent stays registered, and releases it
//! when the agent goes away. While another instance holds it, the relay
//! answers this instance's pending pulls with 409 — so it cannot consume the
//! other instance's jekts — and this module reports the agent as held
//! elsewhere, which fences the local holder (the newcomer yields, D1).
//!
//! Fails open everywhere (D2): a relay without the lease routes (404), an
//! unreachable relay or a transient error never stops delivery or spawning.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How often a held lease is renewed. The relay's TTL is 60 s.
pub(crate) const RENEW_EVERY: Duration = Duration::from_secs(20);

/// A relay answering 404 on the lease routes predates them: stop asking for
/// this long.
const UNSUPPORTED_BACKOFF: Duration = Duration::from_secs(600);

/// How long a "held elsewhere" answer stays good enough to refuse a spawn
/// without asking again.
const HELD_ELSEWHERE_FRESH: Duration = Duration::from_secs(90);

/// This AgentMux instance's id at the relay: host + channel. Stable across
/// restarts of the same install, distinct between two installs on one host.
pub(crate) fn instance_id() -> String {
    format!(
        "{}/{}",
        crate::backend::reactive::registry::local_host_label(),
        crate::backend::reactive::registry::local_channel_id()
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// This instance holds the agent's lease.
    Held,
    /// Another instance holds it; the text describes where.
    HeldElsewhere(String),
    /// No answer (relay unreachable, too old, an error): fail open.
    Unknown,
}

#[derive(Default)]
struct Entry {
    held: bool,
    last_ok: Option<Instant>,
    elsewhere: Option<(String, Instant)>,
    /// When a Take over last cleared `elsewhere`: a 409 to a request that
    /// started before this is stale ([`note_not_holder_since`]).
    forgotten_at: Option<Instant>,
}

static STATE: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);
/// One claim, renewal or release at a time per agent. Without it a
/// release still in flight (the RemoveAgent arm's runs detached, after a
/// token refresh) could land after the agent was subscribed and claimed
/// again, and release that fresh claim (Codex P2 on #3897).
static OPS: LazyLock<Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Default::default);

fn op_lock(agent_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    OPS.lock().unwrap_or_else(|e| e.into_inner()).entry(key(agent_id)).or_default().clone()
}

static UNSUPPORTED_UNTIL: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(Default::default);

fn key(agent_id: &str) -> String {
    agent_id.trim().to_lowercase()
}

/// `Some(where)` when a recent answer said another instance holds `agent`.
/// The early admission check refuses on it without a network round trip.
pub(crate) fn held_elsewhere(agent: &str) -> Option<String> {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let (desc, at) = state.get(&key(agent))?.elsewhere.clone()?;
    (at.elapsed() < HELD_ELSEWHERE_FRESH).then_some(desc)
}

/// Drop a cached "held elsewhere" answer for `agent` — after a Take over
/// freed it, so the retried turn isn't refused from the cache for up to
/// [`HELD_ELSEWHERE_FRESH`] (Codex P1 on #3899).
pub(crate) async fn forget_elsewhere(agent: &str) {
    // Under the per-agent op lock: a lease check already in flight records
    // its answer first, so a late pre-release 409 can't restore the
    // refusal after it was forgotten (Codex P2 on #3903).
    let op = op_lock(agent);
    let _op = op.lock().await;
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let e = state.entry(key(agent)).or_default();
    e.elsewhere = None;
    e.forgotten_at = Some(Instant::now());
}

/// Is a [`describe_holder`] description about this computer (another
/// AgentMux instance on the same host, e.g. a second channel)?
pub(crate) fn holder_on_this_computer(desc: &str) -> bool {
    desc == THIS_COMPUTER || desc.starts_with("this computer, ")
}

const THIS_COMPUTER: &str = "this computer";

/// The channel of another instance on this computer that, per a recent
/// relay answer, holds `agent` — what Take over needs to find it.
pub(crate) fn held_on_this_computer_by(agent: &str) -> Option<String> {
    let desc = held_elsewhere(agent)?;
    if !holder_on_this_computer(&desc) {
        return None;
    }
    desc.split(", ").find_map(|part| part.strip_prefix("channel ")).map(str::to_string)
}

/// Where the holder is when a recent relay answer put `agent` on another
/// computer (a [`describe_holder`] description), else `None`.
pub(crate) fn held_on_another_computer(agent: &str) -> Option<String> {
    held_elsewhere(agent).filter(|desc| !desc.is_empty() && !holder_on_this_computer(desc))
}

/// Describe the relay's `held_by` for a refusal: "computer X, channel Y, vZ".
pub(crate) fn describe_holder(held_by: &serde_json::Value) -> String {
    let field = |k: &str| held_by.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mut parts = Vec::new();
    let host = field("host");
    if !host.is_empty() {
        // The relay only knows the holder's host label; the same label as
        // ours is another instance here, not another computer.
        if host.eq_ignore_ascii_case(&crate::backend::reactive::registry::local_host_label()) {
            parts.push(THIS_COMPUTER.to_string());
        } else {
            parts.push(format!("computer {host}"));
        }
    }
    let channel = field("channel");
    if !channel.is_empty() {
        parts.push(format!("channel {channel}"));
    }
    let version = field("version");
    if !version.is_empty() {
        parts.push(format!("v{version}"));
    }
    parts.join(", ")
}

fn record(agent_id: &str, outcome: &Outcome) {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let e = state.entry(key(agent_id)).or_default();
    match outcome {
        Outcome::Held => {
            e.held = true;
            e.last_ok = Some(Instant::now());
            e.elsewhere = None;
        }
        Outcome::HeldElsewhere(desc) => {
            e.held = false;
            e.last_ok = None;
            e.elsewhere = Some((desc.clone(), Instant::now()));
        }
        Outcome::Unknown => {}
    }
}

/// [`note_not_holder`] for a request that started at `started`: `None`
/// (nothing recorded, and the caller must not fence) when a Take over
/// cleared the refusal after the request started — its 409 predates the
/// holder letting go (Codex P1 on #3908). Pulls and acks don't take the op
/// lock, so ordering by start time is what keeps them from restoring it.
pub(crate) fn note_not_holder_since(agent_id: &str, body: &serde_json::Value, started: Instant) -> Option<String> {
    let desc = body.get("held_by").map(describe_holder).unwrap_or_default();
    // Check and write under one STATE lock (ReAgent P1 on #3908): with
    // two, a forget_elsewhere in between would be overwritten.
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let e = state.entry(key(agent_id)).or_default();
    if e.forgotten_at.is_some_and(|at| at > started) {
        return None;
    }
    e.held = false;
    e.last_ok = None;
    e.elsewhere = Some((desc.clone(), Instant::now()));
    Some(desc)
}

/// Whether this instance currently records `agent_id`'s lease as held.
pub(crate) fn is_held(agent_id: &str) -> bool {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).get(&key(agent_id)).is_some_and(|e| e.held)
}

#[cfg(test)]
pub(crate) fn mark_held_for_test(agent_id: &str) {
    record(agent_id, &Outcome::Held);
}

/// Note a 409 `not_holder` the relay returned on a pending pull or an ack.
pub(crate) fn note_not_holder(agent_id: &str, body: &serde_json::Value) -> String {
    let desc = body.get("held_by").map(describe_holder).unwrap_or_default();
    record(agent_id, &Outcome::HeldElsewhere(desc.clone()));
    desc
}

/// Claim or renew `agent_id`'s WAN lease, at most every [`RENEW_EVERY`].
/// `base` is the relay's REST base URL ([`super::relay::rest_base_url`]).
pub(crate) async fn ensure(
    base: &str,
    agent_id: &str,
    token: &str,
    http: &reqwest::Client,
    still_subscribed: impl Fn() -> bool,
) -> Outcome {
    if UNSUPPORTED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t > Instant::now()) {
        return Outcome::Unknown;
    }
    let op = op_lock(agent_id);
    let _op = op.lock().await;
    // Callers work from a copy of the subscription; a Take over may have
    // released the agent since. Re-checked under the op lock so it
    // can't claim the lease straight back (Codex P2 on #3899).
    if !still_subscribed() {
        return Outcome::Unknown;
    }
    let renew_due = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        match state.get(&key(agent_id)) {
            Some(e) if e.held => match e.last_ok {
                Some(at) if at.elapsed() < RENEW_EVERY => return Outcome::Held,
                _ => true,
            },
            _ => false,
        }
    };
    let outcome = if renew_due {
        match call(base, agent_id, token, http, "/agents/lease/renew").await {
            // Nobody holds it any more (`held_by: null`): claim again.
            Some((409, body)) if body.get("held_by").is_some_and(|h| h.is_null()) => {
                claim_outcome(call(base, agent_id, token, http, "/agents/lease").await)
            }
            other => claim_outcome(other),
        }
    } else {
        claim_outcome(call(base, agent_id, token, http, "/agents/lease").await)
    };
    record(agent_id, &outcome);
    outcome
}

fn claim_outcome(resp: Option<(u16, serde_json::Value)>) -> Outcome {
    match resp {
        Some((200..=299, _)) => Outcome::Held,
        Some((409, body)) => Outcome::HeldElsewhere(body.get("held_by").map(describe_holder).unwrap_or_default()),
        Some((404, _)) | Some((405, _)) => {
            *UNSUPPORTED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + UNSUPPORTED_BACKOFF);
            tracing::info!("wan_lease: relay has no agent-lease routes yet — WAN tier inactive for now");
            Outcome::Unknown
        }
        _ => Outcome::Unknown,
    }
}

/// POST `path` for `agent_id`; `(status, json body)` or `None` on transport error.
async fn call(base: &str, agent_id: &str, token: &str, http: &reqwest::Client, path: &str) -> Option<(u16, serde_json::Value)> {
    let body = serde_json::json!({
        "agent_id": agent_id,
        "instance": instance_id(),
        "host": crate::backend::reactive::registry::local_host_label(),
        "channel": crate::backend::reactive::registry::local_channel_id(),
        "version": env!("CARGO_PKG_VERSION"),
    });
    let resp = http
        .post(format!("{base}{path}"))
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Agent-ID", agent_id)
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let json = r.json::<serde_json::Value>().await.unwrap_or(serde_json::Value::Null);
            Some((status, json))
        }
        Err(e) => {
            tracing::debug!(agent_id, path, error = %e, "wan_lease: relay unreachable");
            None
        }
    }
}

/// Release `agent_id`'s lease (best effort) and forget it — unless
/// `still_gone` says it was subscribed again by the time this runs, in
/// which case this returns `false` (it did not let go).
pub(crate) async fn release(
    base: &str,
    agent_id: &str,
    token: &str,
    http: &reqwest::Client,
    still_gone: impl Fn() -> bool,
) -> bool {
    let op = op_lock(agent_id);
    let _op = op.lock().await;
    // Checked under the op lock: an agent subscribed again since the
    // release was queued keeps its (possibly fresh) claim.
    if !still_gone() {
        return false;
    }
    let was_held = STATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key(agent_id))
        .is_some_and(|e| e.held);
    if was_held {
        let _ = call(base, agent_id, token, http, "/agents/lease/release").await;
    }
    true
}

/// Take over's relay step, after the holder answered its release request:
/// claim `agent_id`'s lease now, and if the relay still names an instance
/// **on this computer**, move it with `POST /agents/lease/take` (the relay
/// allows that only within one account). A holder older than #3899 answers
/// the release but keeps renewing the lease for as long as it runs, so
/// without this the new pane is fenced seconds after it starts
/// (`docs/investigations/INVESTIGATION_TAKE_OVER_OLD_HOLDER_RELAY_LEASE_2026_09_27.md`).
/// Another computer is never taken from: Take over only works on this one.
pub(crate) async fn take_over(base: &str, agent_id: &str, token: &str, http: &reqwest::Client) -> Outcome {
    let op = op_lock(agent_id);
    let _op = op.lock().await;
    let claimed = claim_outcome(call(base, agent_id, token, http, "/agents/lease").await);
    let outcome = match claimed {
        Outcome::HeldElsewhere(desc) if holder_on_this_computer(&desc) => {
            match call(base, agent_id, token, http, "/agents/lease/take").await {
                Some((200..=299, _)) => {
                    tracing::info!(agent_id, from = %desc, "wan_lease: took the lease over from an instance on this computer");
                    Outcome::Held
                }
                // Refused (409) keeps the holder the relay names now; a relay
                // without the route (404/405) or no answer keeps the one we
                // already have. Neither turns the lease tier off.
                Some((409, body)) => claim_outcome(Some((409, body))),
                other => {
                    tracing::warn!(agent_id, answer = ?other.map(|(s, _)| s), "wan_lease: the relay would not take the lease over");
                    Outcome::HeldElsewhere(desc)
                }
            }
        }
        other => other,
    };
    record(agent_id, &outcome);
    outcome
}

/// The relay's answer to "which install holds `agent`'s lease?" —
/// `GET /agents/lease/:agent_id` (agentmux-cloud#136,
/// `docs/plans/PLAN_JEKT_LOCAL_FIRST_ROUTING_2026_10_02.md` §7.1). Asked by
/// a *sender* about a message's target, so it never touches this
/// instance's own lease state ([`record`], admission): a read only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Holder {
    /// No live lease.
    Free,
    /// This instance holds it.
    Yours,
    /// Another install of the same account holds it: the agent is live there.
    Other(String),
    /// A lease the relay won't describe (another account, or none), or a
    /// relay-side error.
    Unknown,
    /// The relay predates the route (404/405).
    Unsupported,
    /// No answer at all.
    Unreachable,
}

/// How long an answer is reused. Shorter than the lease TTL (60 s), so a
/// lease that lapsed or moved is seen within a renewal or two.
const HOLDER_FRESH: Duration = Duration::from_secs(20);

static HOLDER_CACHE: LazyLock<Mutex<HashMap<String, (Holder, Instant)>>> = LazyLock::new(Default::default);
static HOLDER_UNSUPPORTED_UNTIL: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(Default::default);

/// Ask the relay who holds `agent`'s lease, as `caller` (the message's
/// sender, whose credential `token` is). Cached for [`HOLDER_FRESH`];
/// "unreachable" is never cached.
pub(crate) async fn holder(base: &str, agent: &str, caller: &str, token: &str, http: &reqwest::Client) -> Holder {
    if HOLDER_UNSUPPORTED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t > Instant::now()) {
        return Holder::Unsupported;
    }
    if let Some((answer, at)) = HOLDER_CACHE.lock().unwrap_or_else(|e| e.into_inner()).get(&key(agent)) {
        if at.elapsed() < HOLDER_FRESH {
            return answer.clone();
        }
    }
    let Ok(mut url) = url::Url::parse(base) else {
        return Holder::Unknown;
    };
    if let Ok(mut path) = url.path_segments_mut() {
        path.pop_if_empty().extend(["agents", "lease", agent.trim()]);
    }
    let resp = http
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Agent-ID", caller)
        .header("X-Agent-Instance", instance_id())
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    let answer = match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.json::<serde_json::Value>().await.unwrap_or(serde_json::Value::Null);
            holder_from(status, &body)
        }
        Err(e) => {
            tracing::debug!(agent, error = %e, "wan_lease: holder query unreachable");
            return Holder::Unreachable;
        }
    };
    if answer == Holder::Unsupported {
        *HOLDER_UNSUPPORTED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + UNSUPPORTED_BACKOFF);
        tracing::info!("wan_lease: relay has no lease holder query yet — routing as before");
    }
    HOLDER_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key(agent), (answer.clone(), Instant::now()));
    answer
}

fn holder_from(status: u16, body: &serde_json::Value) -> Holder {
    match status {
        200..=299 => match body.get("state").and_then(|s| s.as_str()) {
            Some("free") => Holder::Free,
            Some("yours") => Holder::Yours,
            Some("other") => Holder::Other(body.get("held_by").map(describe_holder).unwrap_or_default()),
            _ => Holder::Unknown,
        },
        404 | 405 => Holder::Unsupported,
        _ => Holder::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The holder query on the wire: `GET {base}/agents/lease/{agent}` with
    /// the sender's credential, its agent id and this instance's id; the
    /// answer is reused for HOLDER_FRESH, and "unreachable" is not cached.
    #[tokio::test]
    async fn the_holder_query_asks_once_and_names_the_caller() {
        let hits = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, Option<String>, Option<String>, Option<String>)>::new()));
        let sink = hits.clone();
        let app = axum::Router::new().route(
            "/agents/lease/:agent",
            axum::routing::get(
                move |axum::extract::Path(agent): axum::extract::Path<String>, headers: axum::http::HeaderMap| {
                    let sink = sink.clone();
                    async move {
                        let get = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
                        sink.lock().unwrap().push((agent, get("authorization"), get("x-agent-id"), get("x-agent-instance")));
                        axum::Json(serde_json::json!({ "state": "other", "held_by": { "host": "area54-holder-test", "channel": "stable" } }))
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let base = format!("http://{addr}");
        let http = reqwest::Client::new();
        let agent = format!("HolderTest-{}", &uuid::Uuid::new_v4().to_string()[..8]);

        let first = holder(&base, &agent, "lark", "tok", &http).await;
        assert!(matches!(&first, Holder::Other(d) if d.contains("area54-holder-test")), "{first:?}");
        let again = holder(&base, &agent, "lark", "tok", &http).await;
        assert_eq!(again, first);
        let seen = hits.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "the second answer came from the cache: {seen:?}");
        let (path_agent, auth, caller, instance) = &seen[0];
        assert_eq!(path_agent, &agent);
        assert_eq!(auth.as_deref(), Some("Bearer tok"));
        assert_eq!(caller.as_deref(), Some("lark"));
        assert_eq!(instance.as_deref(), Some(instance_id().as_str()));

        // Nothing listening: unreachable, and asked again next time.
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let lost = format!("HolderLost-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        assert_eq!(holder(&dead, &lost, "lark", "tok", &http).await, Holder::Unreachable);
        assert!(HOLDER_CACHE.lock().unwrap().get(&key(&lost)).is_none());
    }

    /// The holder query's answers (agentmux-cloud#136). Anything the relay
    /// does not say plainly is `Unknown`, which routes as before.
    #[test]
    fn holder_answers_map_to_routes() {
        use serde_json::json;
        assert_eq!(holder_from(200, &json!({ "state": "free" })), Holder::Free);
        assert_eq!(holder_from(200, &json!({ "state": "yours" })), Holder::Yours);
        let other = holder_from(200, &json!({ "state": "other", "held_by": { "host": "area54", "channel": "stable", "version": "0.59.5" } }));
        assert!(matches!(&other, Holder::Other(d) if d.contains("area54")), "{other:?}");
        assert_eq!(holder_from(200, &json!({ "state": "unknown" })), Holder::Unknown);
        assert_eq!(holder_from(200, &json!({ "state": "weird" })), Holder::Unknown);
        assert_eq!(holder_from(200, &serde_json::Value::Null), Holder::Unknown);
        assert_eq!(holder_from(404, &serde_json::Value::Null), Holder::Unsupported);
        assert_eq!(holder_from(405, &serde_json::Value::Null), Holder::Unsupported);
        for status in [401, 403, 429, 500] {
            assert_eq!(holder_from(status, &serde_json::Value::Null), Holder::Unknown, "{status}");
        }
    }

    // Codex P1 on #3899: after a successful Take over the requester's own
    // cached refusal still refused the retried turn for up to 90 s.
    #[tokio::test]
    async fn forgetting_a_refusal_lets_the_next_turn_through() {
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        note_not_holder(&agent, &serde_json::json!({ "held_by": { "host": "desk" } }));
        assert!(held_elsewhere(&agent).is_some());
        forget_elsewhere(&agent.to_uppercase()).await;
        assert_eq!(held_elsewhere(&agent), None);
    }

    // Codex P1 on #3908: a pull or ack answered 409 before the holder let
    // go, but processed after the refusal was forgotten, restored it (and
    // could fence the new pane). Only answers to requests started after
    // the forget count.
    #[tokio::test]
    async fn a_409_to_a_request_from_before_a_take_over_is_ignored() {
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        let body = serde_json::json!({ "held_by": { "host": "desk" } });
        let started = Instant::now();
        tokio::time::sleep(Duration::from_millis(5)).await;
        forget_elsewhere(&agent).await;
        assert_eq!(note_not_holder_since(&agent, &body, started), None, "stale: not recorded, not fenced");
        assert_eq!(held_elsewhere(&agent), None);

        let later = Instant::now();
        assert!(note_not_holder_since(&agent, &body, later).is_some(), "a fresh 409 still counts");
        assert!(held_elsewhere(&agent).is_some());
    }

    // Codex P2 on #3903: a lease check already in flight could record a
    // delayed pre-release 409 after the refusal was forgotten, restoring it
    // for another 90 s. Forgetting waits for it.
    #[tokio::test]
    async fn forgetting_a_refusal_waits_for_a_lease_check_in_flight() {
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        let lock = op_lock(&agent);
        let check = lock.lock().await;
        let a = agent.clone();
        let forget = tokio::spawn(async move { forget_elsewhere(&a).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!forget.is_finished(), "forgetting waits for the check");
        // The in-flight check lands its (stale) 409.
        note_not_holder(&agent, &serde_json::json!({ "held_by": { "host": "desk" } }));
        drop(check);
        tokio::time::timeout(Duration::from_secs(5), forget).await.unwrap().unwrap();
        assert_eq!(held_elsewhere(&agent), None, "the stale refusal is gone");
    }

    // Codex P2 on #3899: a lease tick that snapshotted the agent before a
    // Take over released it would claim the lease straight back.
    #[tokio::test]
    async fn a_renewal_for_an_agent_no_longer_subscribed_claims_nothing() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        let app = axum::Router::new().fallback(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                axum::Json(serde_json::json!({ "ok": true, "epoch": 1 }))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        let http = reqwest::Client::new();

        assert_eq!(ensure(&base, &agent, "", &http, || false).await, Outcome::Unknown);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0, "no claim for an unsubscribed agent");
        assert_eq!(ensure(&base, &agent, "", &http, || true).await, Outcome::Held);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // Codex P2 (third pass) on #3897: the RemoveAgent arm's release runs
    // detached; if the agent was subscribed and claimed again meanwhile, it
    // released that fresh claim and let another instance fence the agent.
    #[tokio::test]
    async fn a_release_skips_an_agent_subscribed_again() {
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        record(&agent, &Outcome::Held);
        assert!(
            !release("http://127.0.0.1:9", &agent, "", &reqwest::Client::new(), || false).await,
            "reports that it did not let go"
        );
        let held = STATE.lock().unwrap().get(&key(&agent)).is_some_and(|e| e.held);
        assert!(held, "the fresh claim is left alone");
    }

    #[tokio::test]
    async fn a_release_waits_for_a_claim_in_progress_for_the_same_agent() {
        let agent = format!("Agent-{}", uuid::Uuid::new_v4());
        let lock = op_lock(&agent.to_lowercase());
        let claim = lock.lock().await;
        let a = agent.clone();
        let release = tokio::spawn(async move {
            super::release("http://127.0.0.1:9", &a, "", &reqwest::Client::new(), || true).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!release.is_finished(), "the release waits for the claim");
        drop(claim);
        tokio::time::timeout(Duration::from_secs(5), release).await.unwrap().unwrap();
    }

    #[test]
    fn the_holder_is_described_without_account_or_instance() {
        let held_by = serde_json::json!({ "host": "desk", "channel": "stable", "version": "0.58.0", "acquired_at": 1 });
        assert_eq!(describe_holder(&held_by), "computer desk, channel stable, v0.58.0");
        assert_eq!(describe_holder(&serde_json::Value::Null), "");
    }

    // Charlie 2026-09-26: 0.57.6 and a 0.57.7 build on the same computer;
    // the refusal said "on another computer" about its own host.
    #[test]
    fn a_holder_on_this_computer_is_described_as_this_computer() {
        let here = crate::backend::reactive::registry::local_host_label();
        let held_by = serde_json::json!({ "host": here, "channel": "local-main-x", "version": "0.57.6" });
        assert_eq!(describe_holder(&held_by), "this computer, channel local-main-x, v0.57.6");
        assert!(holder_on_this_computer(&describe_holder(&held_by)));
        assert!(!holder_on_this_computer("computer desk, channel stable, v0.58.0"));
    }

    // Take over needs the channel of a same-computer holder the relay
    // reported, to find that instance and ask it to let go.
    #[test]
    fn a_same_computer_holder_gives_take_over_its_channel() {
        let here = crate::backend::reactive::registry::local_host_label();
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        note_not_holder(&agent, &serde_json::json!({ "held_by": { "host": here, "channel": "local-main-x", "version": "0.57.6" } }));
        assert_eq!(held_on_this_computer_by(&agent).as_deref(), Some("local-main-x"));
        assert_eq!(held_on_another_computer(&agent), None);

        let other = format!("agent-{}", uuid::Uuid::new_v4());
        note_not_holder(&other, &serde_json::json!({ "held_by": { "host": "desk", "channel": "stable" } }));
        assert_eq!(held_on_this_computer_by(&other), None);
        assert_eq!(held_on_another_computer(&other).as_deref(), Some("computer desk, channel stable"));

        assert_eq!(held_on_this_computer_by("never-seen"), None);
    }

    /// A relay stub for [`take_over`]: `claim` and `take` answer with the
    /// given status and body; each route counts its hits.
    async fn lease_stub(
        claim: (u16, serde_json::Value),
        take: (u16, serde_json::Value),
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (claims, takes) = (std::sync::Arc::new(AtomicUsize::new(0)), std::sync::Arc::new(AtomicUsize::new(0)));
        let (c, t) = (claims.clone(), takes.clone());
        let answer = |(status, body): (u16, serde_json::Value)| {
            (axum::http::StatusCode::from_u16(status).unwrap(), axum::Json(body))
        };
        let app = axum::Router::new()
            .route(
                "/agents/lease",
                axum::routing::post(move || {
                    c.fetch_add(1, Ordering::SeqCst);
                    let r = answer(claim.clone());
                    async move { r }
                }),
            )
            .route(
                "/agents/lease/take",
                axum::routing::post(move || {
                    t.fetch_add(1, Ordering::SeqCst);
                    let r = answer(take.clone());
                    async move { r }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, claims, takes)
    }

    fn held_here(channel: &str) -> serde_json::Value {
        let here = crate::backend::reactive::registry::local_host_label();
        serde_json::json!({ "error": "held_by_other", "held_by": { "host": here, "channel": channel, "version": "0.57.6" } })
    }

    // narko 2026-09-27: a 0.57.6 holder answered the release but kept
    // renewing the relay lease; Take over has to move it itself.
    #[tokio::test]
    async fn take_over_takes_a_lease_an_instance_on_this_computer_kept() {
        use std::sync::atomic::Ordering;
        let (base, claims, takes) =
            lease_stub((409, held_here("local-main-old")), (200, serde_json::json!({ "ok": true, "epoch": 6 }))).await;
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        assert_eq!(take_over(&base, &agent, "", &reqwest::Client::new()).await, Outcome::Held);
        assert_eq!((claims.load(Ordering::SeqCst), takes.load(Ordering::SeqCst)), (1, 1));
        assert!(is_held(&agent), "recorded, so the lease tick renews it");
        assert_eq!(held_elsewhere(&agent), None);
    }

    #[tokio::test]
    async fn take_over_never_takes_from_another_computer() {
        use std::sync::atomic::Ordering;
        let other = serde_json::json!({ "error": "held_by_other", "held_by": { "host": "some-other-desk", "channel": "stable" } });
        let (base, _, takes) = lease_stub((409, other), (200, serde_json::json!({ "ok": true, "epoch": 6 }))).await;
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        assert_eq!(
            take_over(&base, &agent, "", &reqwest::Client::new()).await,
            Outcome::HeldElsewhere("computer some-other-desk, channel stable".into())
        );
        assert_eq!(takes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn take_over_of_a_free_lease_is_a_plain_claim() {
        use std::sync::atomic::Ordering;
        let (base, _, takes) = lease_stub((200, serde_json::json!({ "ok": true, "epoch": 1 })), (500, serde_json::Value::Null)).await;
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        assert_eq!(take_over(&base, &agent, "", &reqwest::Client::new()).await, Outcome::Held);
        assert_eq!(takes.load(Ordering::SeqCst), 0);
    }

    // A relay deployed before /agents/lease/take answers it 404. That must
    // keep the holder's refusal, and must not switch the whole lease tier
    // off the way a 404 on the claim route does.
    #[tokio::test]
    async fn take_over_on_a_relay_without_take_keeps_the_refusal_and_the_lease_tier() {
        let (base, _, _) = lease_stub((409, held_here("local-main-old")), (404, serde_json::Value::Null)).await;
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        let outcome = take_over(&base, &agent, "", &reqwest::Client::new()).await;
        assert!(matches!(&outcome, Outcome::HeldElsewhere(d) if holder_on_this_computer(d)), "{outcome:?}");
        assert!(
            UNSUPPORTED_UNTIL.lock().unwrap().is_none_or(|t| t <= Instant::now()),
            "a missing take route is not a missing lease tier"
        );
    }

    #[test]
    fn claim_answers_map_to_outcomes() {
        assert_eq!(claim_outcome(Some((200, serde_json::json!({ "ok": true, "epoch": 1 })))), Outcome::Held);
        assert_eq!(
            claim_outcome(Some((409, serde_json::json!({ "held_by": { "host": "desk" } })))),
            Outcome::HeldElsewhere("computer desk".into())
        );
        assert_eq!(claim_outcome(None), Outcome::Unknown, "unreachable relay: fail open");
        assert_eq!(claim_outcome(Some((500, serde_json::Value::Null))), Outcome::Unknown);
    }

    #[test]
    fn a_not_holder_answer_is_remembered_for_the_early_check_and_cleared_by_a_grant() {
        let agent = format!("agent-{}", uuid::Uuid::new_v4());
        assert!(held_elsewhere(&agent).is_none());
        let desc = note_not_holder(&agent.to_uppercase(), &serde_json::json!({ "held_by": { "host": "desk" } }));
        assert_eq!(desc, "computer desk");
        assert_eq!(held_elsewhere(&agent).as_deref(), Some("computer desk"), "keyed case-insensitively, like the relay");
        record(&agent, &Outcome::Held);
        assert!(held_elsewhere(&agent).is_none());
    }

    #[test]
    fn the_instance_id_is_host_and_channel() {
        let id = instance_id();
        assert_eq!(id.matches('/').count(), 1, "{id}");
        assert!(id.ends_with(&crate::backend::reactive::registry::local_channel_id()));
    }
}
