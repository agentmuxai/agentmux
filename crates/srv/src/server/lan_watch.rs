// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One WARN line for an incoming LAN request that is refused, hangs or times out.
//!
//! On 2026-11-12 a machine refused or never answered every remote operation from another for 17
//! hours, and its log had nothing about it: srv logs inbound LAN HTTP at INFO only when a request
//! is delivered, so the one request that matters, the one that never gets through, left no line.
//! A middleware on the three routes a LAN peer posts to (`/agentmux/reactive/inject`,
//! `/agentmux/remote/op`, `/agentmux/reactive/inject-files`) writes the line, and handlers mark
//! the stage they are at ([`stage`]) where they wait on something, so a hang names what it waits on.
//!
//! A line names the route, the peer's address and machine, the agent the request *claims* to come
//! from (nothing is proven yet at most stages), the stage, how long, and why. Never a request body,
//! a key or a token. A peer retrying every few seconds writes one line a minute per reason, with
//! how many it stood for. `App health` carries the counts since start (`lan_refusals`).

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;
use serde_json::{json, Value};

use super::AppState;

/// The routes watched, and how long each may go unanswered before a line says so. A file transfer
/// streams for as long as its files take, so its wait is far longer than a jekt's.
const WATCHED: [(&str, Duration); 3] = [
    ("/agentmux/reactive/inject", Duration::from_secs(5)),
    ("/agentmux/remote/op", Duration::from_secs(5)),
    ("/agentmux/reactive/inject-files", Duration::from_secs(60)),
];

/// A line per source and reason at most this often; the ones left out are counted on the next.
const LINE_EVERY_SECS: i64 = 60;
/// Source-and-reason pairs remembered; a peer that makes up reasons cannot grow the table.
const MAX_KEYS: usize = 256;
/// The longest answer read to see why it refused: the answers of these routes are a line of JSON.
const MAX_ANSWER_BYTES: usize = 64 * 1024;
/// What a reason or stage keeps of the text it was made from.
const MAX_TEXT_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Answered, with a refusal.
    Refused,
    /// Still being handled after the route's wait; the answer, if any, comes later.
    Unanswered,
    /// The handler gave up waiting on something.
    TimedOut,
}

impl Kind {
    fn words(self) -> &'static str {
        match self {
            Kind::Refused => "refused",
            Kind::Unanswered => "still unanswered",
            Kind::TimedOut => "timed out",
        }
    }
}

/// What one watched request has said about itself so far.
struct Watch {
    stage: Mutex<String>,
    claimed: Mutex<String>,
    timed_out: Mutex<bool>,
}

tokio::task_local! {
    static WATCH: Arc<Watch>;
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `text` as one short line a log can carry: printable ASCII and a few separators, the rest an
/// underscore, cut at [`MAX_TEXT_CHARS`].
fn clean(text: &str) -> String {
    text.trim()
        .chars()
        .take(MAX_TEXT_CHARS)
        .map(|c| if c.is_ascii_alphanumeric() || " ._-@:/,()'".contains(c) { c } else { '_' })
        .collect()
}

/// The request being handled is at `name`: the check or wait it is in (the last one entered, so a refusal
/// later on can still name an earlier one). Does nothing outside a watched request.
pub(crate) fn stage(name: &str) {
    let _ = WATCH.try_with(|w| *lock(&w.stage) = clean(name));
}

/// The agent the request says it comes from, unproven. Does nothing outside a watched request.
pub(crate) fn claimed(agent: &str) {
    let _ = WATCH.try_with(|w| *lock(&w.claimed) = clean(agent));
}

/// `refusal`, after noting that the handler gave up waiting: the line says timed out, not refused.
pub(crate) fn timed_out<T>(refusal: T) -> T {
    let _ = WATCH.try_with(|w| *lock(&w.timed_out) = true);
    refusal
}

/// `fut`, run with the request marked as being at `name`: the check or wait it is in.
pub(crate) async fn during<F: Future>(name: &str, fut: F) -> F::Output {
    stage(name);
    fut.await
}

/// [`during`] "actor check", for a request that claims to come from `agent`.
pub(crate) async fn acting<F: Future>(agent: Option<&str>, fut: F) -> F::Output {
    claimed(agent.unwrap_or(""));
    during("actor check", fut).await
}

/// `fut`, run in the watched request this is called from, if any: a task spawned for the request
/// otherwise would lose it and its stages would go unseen.
pub(crate) fn carry<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    let current = WATCH.try_with(Arc::clone).ok();
    async move {
        match current {
            Some(watch) => WATCH.scope(watch, fut).await,
            None => fut.await,
        }
    }
}

// ── Rate limit ───────────────────────────────────────────────────────────────

/// When a line was last written for each source and reason, and how many were left out since.
#[derive(Default)]
struct Limits {
    last: HashMap<String, (i64, u32)>,
}

impl Limits {
    /// `Some(n)` when a line is due for `key` at `now`: `n` is this one and every one left out since the
    /// last. `None` when one was written less than a minute ago (this one is counted for the next).
    fn due(&mut self, key: &str, now: i64) -> Option<u32> {
        if let Some((at, left_out)) = self.last.get_mut(key) {
            if now - *at < LINE_EVERY_SECS {
                *left_out += 1;
                return None;
            }
            let count = 1 + *left_out;
            *at = now;
            *left_out = 0;
            return Some(count);
        }
        if self.last.len() >= MAX_KEYS {
            // Entries past their minute with nothing left out are the ones to forget.
            self.last.retain(|_, (at, left_out)| now - *at < LINE_EVERY_SECS || *left_out > 0);
            if self.last.len() >= MAX_KEYS {
                return None;
            }
        }
        self.last.insert(key.to_string(), (now, 0));
        Some(1)
    }
}

static LIMITS: LazyLock<Mutex<Limits>> = LazyLock::new(Default::default);

static REFUSED: AtomicU64 = AtomicU64::new(0);
static UNANSWERED: AtomicU64 = AtomicU64::new(0);
static TIMED_OUT: AtomicU64 = AtomicU64::new(0);

/// What `App health` reports: requests of each kind since this srv started, counted before the line limit.
pub(crate) fn counts() -> Value {
    json!({
        "refused": REFUSED.load(Ordering::Relaxed),
        "unanswered": UNANSWERED.load(Ordering::Relaxed),
        "timed_out": TIMED_OUT.load(Ordering::Relaxed),
    })
}

// ── The line ─────────────────────────────────────────────────────────────────

/// Everything one line says.
struct Event<'a> {
    kind: Kind,
    route: &'a str,
    address: String,
    machine: String,
    agent: String,
    stage: String,
    reason: String,
    took_ms: u64,
}

impl Event<'_> {
    /// The line, with how many requests it stands for when more than one.
    fn message(&self, count: u32) -> String {
        let agent = if self.agent.is_empty() { "no agent named".to_string() } else { format!("claiming to be agent {}", self.agent) };
        let mut line = format!(
            "incoming LAN request {}: {} from {} ({}), {}, at stage \"{}\" after {} ms",
            self.kind.words(),
            self.route,
            self.machine,
            self.address,
            agent,
            self.stage,
            self.took_ms,
        );
        if !self.reason.is_empty() {
            line.push_str(&format!(", reason: {}", self.reason));
        }
        if count > 1 {
            line.push_str(&format!(" ({count} like it in the last minute)"));
        }
        line
    }

    /// What decides whether two events are the same to the limit: one source, one kind, one reason.
    fn limit_key(&self) -> String {
        format!("{}|{:?}|{}|{}", self.address, self.kind, self.route, self.reason)
    }
}

/// Why `status` and the JSON `body` an answer carried say the request was refused or timed out, if they do.
/// The routes answer a refusal as 200 with `success: false` (a jekt) or `status: "refused"` (a remote
/// operation), so the status alone does not tell.
fn classify(status: u16, body: &[u8]) -> Option<(Kind, String)> {
    let json: Option<Value> = serde_json::from_slice(body).ok();
    let text = |key: &str| json.as_ref().and_then(|j| j.get(key)).and_then(Value::as_str).map(clean).filter(|s| !s.is_empty());
    if status == 408 || status == 504 {
        return Some((Kind::TimedOut, text("error").unwrap_or_else(|| format!("http {status}"))));
    }
    if status >= 400 {
        return Some((Kind::Refused, text("error").unwrap_or_else(|| format!("http {status}"))));
    }
    let json = json.as_ref()?;
    if json.get("success").and_then(Value::as_bool) == Some(false) {
        return Some((Kind::Refused, text("error").unwrap_or_else(|| "no reason given".into())));
    }
    if json.get("status").and_then(Value::as_str) == Some("refused") {
        return Some((Kind::Refused, text("refused_reason").unwrap_or_else(|| "no reason given".into())));
    }
    None
}

fn watched(path: &str) -> Option<Duration> {
    WATCHED.iter().find(|(route, _)| *route == path).map(|(_, wait)| *wait)
}

/// Count it, and write its line unless one for the same source and reason was written lately.
fn report(state: &AppState, from: Option<SocketAddr>, route: &str, watch: &Watch, kind: Kind, reason: String, started: Instant) {
    let counter = match kind {
        Kind::Refused => &REFUSED,
        Kind::Unanswered => &UNANSWERED,
        Kind::TimedOut => &TIMED_OUT,
    };
    counter.fetch_add(1, Ordering::Relaxed);
    let event = Event {
        kind,
        route,
        address: from.map(|a| a.ip().to_string()).unwrap_or_else(|| "unknown address".into()),
        machine: super::remote_ops::receive::machine_in(&state.lan_discovery.get_all_instances(), from),
        agent: lock(&watch.claimed).clone(),
        stage: lock(&watch.stage).clone(),
        reason,
        took_ms: started.elapsed().as_millis() as u64,
    };
    let Some(count) = lock(&LIMITS).due(&event.limit_key(), agentmux_common::time::now_secs()) else { return };
    let message = event.message(count);
    #[cfg(test)]
    lock(&EMITTED).push(message.clone());
    tracing::warn!(target: "lan-watch", "{message}");
}

#[cfg(test)]
static EMITTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

// ── The middleware ───────────────────────────────────────────────────────────

/// Watches a request to one of [`WATCHED`]: a line when it is refused, unanswered after the route's
/// wait, or its handler timed out. Outside every other layer, so an authentication refusal is seen too.
pub(super) async fn watch(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let route = req.uri().path().to_string();
    let Some(hang_after) = watched(&route) else { return next.run(req).await };
    let from = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let watch = Arc::new(Watch { stage: Mutex::new("request checks".into()), claimed: Mutex::new(String::new()), timed_out: Mutex::new(false) });
    let started = Instant::now();

    let handling = WATCH.scope(watch.clone(), next.run(req));
    tokio::pin!(handling);
    let response = match tokio::time::timeout(hang_after, &mut handling).await {
        Ok(response) => response,
        Err(_) => {
            report(&state, from, &route, &watch, Kind::Unanswered, "no answer yet".into(), started);
            handling.await
        }
    };

    let (response, verdict) = judge(response).await;
    let verdict = match (*lock(&watch.timed_out), verdict) {
        (true, Some((_, reason))) => Some((Kind::TimedOut, reason)),
        (true, None) => Some((Kind::TimedOut, "the operation did not finish in time".into())),
        (false, verdict) => verdict,
    };
    if let Some((kind, reason)) = verdict {
        report(&state, from, &route, &watch, kind, reason, started);
    }
    response
}

/// The answer, unchanged, and why it says refused or timed out, if it does. Its body is read when it is a small
/// JSON one, and put back.
async fn judge(response: Response) -> (Response, Option<(Kind, String)>) {
    let (parts, body) = response.into_parts();
    let status = parts.status.as_u16();
    // The size is the body's own, not a Content-Length header: hyper adds that after the answer leaves here.
    let small_json = parts.headers.get(axum::http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("json"))
        && axum::body::HttpBody::size_hint(&body).upper().is_some_and(|n| n <= MAX_ANSWER_BYTES as u64);
    if !small_json {
        return (Response::from_parts(parts, body), classify(status, &[]));
    }
    match axum::body::to_bytes(body, MAX_ANSWER_BYTES).await {
        Ok(bytes) => {
            let verdict = classify(status, &bytes);
            (Response::from_parts(parts, Body::from(bytes)), verdict)
        }
        Err(_) => (Response::from_parts(parts, Body::empty()), classify(status, &[])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: Kind, stage: &str, reason: &str) -> Event<'static> {
        Event {
            kind,
            route: "/agentmux/remote/op",
            address: "203.0.113.7".into(),
            machine: "narko".into(),
            agent: "agenty".into(),
            stage: stage.into(),
            reason: reason.into(),
            took_ms: 5012,
        }
    }

    #[test]
    fn a_refusal_names_the_route_the_source_the_stage_and_the_reason() {
        let line = event(Kind::Refused, "sender key lookup", "unknown_sender").message(1);
        assert_eq!(
            line,
            "incoming LAN request refused: /agentmux/remote/op from narko (203.0.113.7), claiming to be agent agenty, \
             at stage \"sender key lookup\" after 5012 ms, reason: unknown_sender"
        );
    }

    #[test]
    fn a_request_still_unanswered_names_the_stage_it_waits_in() {
        let line = event(Kind::Unanswered, "sender key lookup", "no answer yet").message(1);
        assert!(line.starts_with("incoming LAN request still unanswered: /agentmux/remote/op from narko"), "{line}");
        assert!(line.contains("at stage \"sender key lookup\" after 5012 ms, reason: no answer yet"), "{line}");
    }

    #[test]
    fn a_timeout_says_so_and_a_request_naming_no_agent_says_that() {
        let mut e = event(Kind::TimedOut, "operation app.health", "the operation did not finish in time");
        e.agent.clear();
        let line = e.message(1);
        assert!(line.starts_with("incoming LAN request timed out: "), "{line}");
        assert!(line.contains("no agent named, at stage \"operation app.health\""), "{line}");
    }

    #[test]
    fn a_line_standing_for_several_says_how_many() {
        assert!(event(Kind::Refused, "x", "y").message(14).ends_with("(14 like it in the last minute)"));
        assert!(!event(Kind::Refused, "x", "y").message(1).contains("like it"));
    }

    #[test]
    fn a_peer_retrying_writes_one_line_a_minute_with_the_count() {
        let mut limits = Limits::default();
        assert_eq!(limits.due("a|refused", 1000), Some(1));
        for second in 1..=59 {
            assert_eq!(limits.due("a|refused", 1000 + second), None, "second {second}");
        }
        // The minute is up: the line says it stands for this one and the 59 left out.
        assert_eq!(limits.due("a|refused", 1060), Some(60));
        assert_eq!(limits.due("a|refused", 1061), None);
    }

    #[test]
    fn each_source_and_reason_has_its_own_line() {
        let mut limits = Limits::default();
        assert_eq!(limits.due("a|refused|x", 1000), Some(1));
        assert_eq!(limits.due("b|refused|x", 1000), Some(1));
        assert_eq!(limits.due("a|timed_out|x", 1000), Some(1));
        assert_eq!(limits.due("a|refused|x", 1001), None);
    }

    #[test]
    fn a_peer_making_up_reasons_cannot_grow_the_table() {
        let mut limits = Limits::default();
        for n in 0..MAX_KEYS {
            assert_eq!(limits.due(&format!("k{n}"), 1000), Some(1));
        }
        assert_eq!(limits.due("one-more", 1001), None, "full, and every entry is within its minute");
        assert_eq!(limits.last.len(), MAX_KEYS);
        // A minute later the quiet ones are forgotten and a new one fits.
        assert_eq!(limits.due("one-more", 1100), Some(1));
        assert!(limits.last.len() <= MAX_KEYS);
    }

    #[test]
    fn a_refused_remote_operation_a_refused_jekt_and_a_bad_status_are_refusals() {
        assert_eq!(classify(200, br#"{"status":"refused","refused_reason":"unknown_sender","audit_id":""}"#), Some((Kind::Refused, "unknown_sender".into())));
        assert_eq!(classify(200, br#"{"success":false,"error":"agent not found: x"}"#), Some((Kind::Refused, "agent not found: x".into())));
        assert_eq!(classify(401, b"unauthorized"), Some((Kind::Refused, "http 401".into())));
        assert_eq!(classify(503, br#"{"success":false,"error":"busy receiving other files"}"#), Some((Kind::Refused, "busy receiving other files".into())));
    }

    #[test]
    fn a_gateway_or_request_timeout_is_a_timeout() {
        assert_eq!(classify(504, b""), Some((Kind::TimedOut, "http 504".into())));
        assert_eq!(classify(408, br#"{"error":"request timed out"}"#), Some((Kind::TimedOut, "request timed out".into())));
    }

    #[test]
    fn a_delivered_jekt_and_a_done_or_failed_operation_are_not_refusals() {
        assert_eq!(classify(200, br#"{"success":true,"request_id":"r"}"#), None);
        assert_eq!(classify(200, br#"{"status":"done","result":{}}"#), None);
        assert_eq!(classify(200, br#"{"status":"failed","result":{"error":"disk full"}}"#), None);
        assert_eq!(classify(200, b"not json"), None);
    }

    #[test]
    fn a_reason_is_one_short_printable_line() {
        let reason = clean("bad\nsecret: \u{1b}[31m token=abc\t");
        assert!(!reason.contains('\n') && !reason.contains('\u{1b}') && !reason.contains('\t'), "{reason:?}");
        assert_eq!(clean(&"x".repeat(500)).chars().count(), MAX_TEXT_CHARS);
    }

    #[tokio::test]
    async fn stages_and_the_claimed_agent_are_kept_for_the_watched_request_only() {
        stage("outside: nothing to keep"); // no request: a no-op, not a panic
        let watch = Arc::new(Watch { stage: Mutex::new("request checks".into()), claimed: Mutex::new(String::new()), timed_out: Mutex::new(false) });
        WATCH
            .scope(watch.clone(), async {
                stage("sender key lookup");
                claimed("Agent Y");
                assert_eq!(timed_out(7), 7);
                // A task spawned for the request keeps it only through `carry`.
                tokio::spawn(carry(async { stage("delivery") })).await.unwrap();
                tokio::spawn(async { stage("lost") }).await.unwrap();
            })
            .await;
        assert_eq!(*lock(&watch.stage), "delivery");
        assert_eq!(*lock(&watch.claimed), "Agent Y");
        assert!(*lock(&watch.timed_out));
    }

    use axum::routing::post;
    use tower::ServiceExt;

    async fn answered(body: serde_json::Value, wait: Duration, at: &'static str, agent: &'static str) -> axum::response::Response {
        let state = crate::server::tests::test_state();
        let app = axum::Router::new()
            .route(
                "/agentmux/remote/op",
                post(move || async move {
                    claimed(agent);
                    stage(at);
                    tokio::time::sleep(wait).await;
                    axum::Json(body)
                }),
            )
            .layer(axum::middleware::from_fn_with_state(state, watch));
        app.oneshot(Request::post("/agentmux/remote/op").body(Body::empty()).unwrap()).await.unwrap()
    }

    fn emitted_for(agent: &str) -> Vec<String> {
        lock(&EMITTED).iter().filter(|m| m.contains(&format!("claiming to be agent {agent}"))).cloned().collect()
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_request_leaves_a_line_and_its_answer_still_reaches_the_peer() {
        let response = answered(json!({"status":"refused","refused_reason":"unverified_sender"}), Duration::ZERO, "sender key lookup", "watch-refused").await;
        let bytes = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["refused_reason"], "unverified_sender");
        let lines = emitted_for("watch-refused");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("refused") && lines[0].contains("at stage \"sender key lookup\"") && lines[0].contains("reason: unverified_sender"), "{lines:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_unanswered_after_its_wait_leaves_a_line_naming_where_it_waits() {
        let response = answered(json!({"status":"done","result":{}}), Duration::from_secs(8), "sender key lookup", "watch-hangs").await;
        assert_eq!(response.status(), 200);
        let lines = emitted_for("watch-hangs");
        assert_eq!(lines.len(), 1, "an answer that came, and was fine, adds no second line: {lines:?}");
        assert!(lines[0].contains("still unanswered") && lines[0].contains("at stage \"sender key lookup\""), "{lines:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_that_is_answered_in_time_and_well_leaves_nothing() {
        let response = answered(json!({"status":"done","result":{}}), Duration::from_secs(1), "operation app.health", "watch-quiet").await;
        assert_eq!(response.status(), 200);
        assert!(emitted_for("watch-quiet").is_empty());
    }

    #[test]
    fn the_watched_routes_are_the_three_a_peer_posts_to() {
        assert_eq!(watched("/agentmux/reactive/inject"), Some(Duration::from_secs(5)));
        assert_eq!(watched("/agentmux/remote/op"), Some(Duration::from_secs(5)));
        assert_eq!(watched("/agentmux/reactive/inject-files"), Some(Duration::from_secs(60)));
        assert_eq!(watched("/agentmux/fleet/events"), None, "a stream is open for as long as its reader is");
    }
}
