// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The browser sign-in of a headless srv that serves its own UI (`--frontend-dir`) on loopback:
//! the `/auth/*` routes, the check that admits a browser's session in [`auth_middleware`], and the
//! headers of the served UI. The sessions themselves are in [`super::local_session`].
//!
//! Every answer to a failed `/auth/code` or `/auth/enter` is the same 403 `not accepted`, after the
//! same short wait, whatever was wrong; the reason goes to the log only, never the secret.
//! The routes are registered here, not in `routes.rs`: they authenticate themselves (a container
//! token never reaches them: its listener refuses a path with no audience).

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, HeaderValue};
use agentmux_common::AUTH_KEY_HEADER;
use serde::Deserialize;

use super::local_session::{CodeState, Collected, RouteClass, SessionId, SessionStore, LOGGED_FAILURES};
use super::*;

/// The cookie's name is this, then the instance id.
const COOKIE_PREFIX: &str = "amx-s-";
/// The per-tab token on a fetch.
pub(crate) const TOKEN_HEADER: &str = "x-am-session";
/// The protocol srv selects on `/ws`; the other protocol the page offers is the token.
pub(crate) const WS_PROTOCOL: &str = "amx-session";
/// A refused sign-in waits this long before it answers, in an async sleep.
const FAILURE_DELAY: Duration = Duration::from_millis(250);
/// At most this many refusals wait at once; beyond it a refusal answers at once.
const MAX_DELAYED: usize = 64;
/// How long a waiting page's request is held before it asks again.
const WAIT_HOLD: Duration = Duration::from_secs(20);
/// Routes that take the cookie alone: GETs that change nothing and that an element loads by URL.
/// The UI fetches every file with its headers, so none needs it; each entry added here is a
/// reviewed decision, and the test in `tests/local_session_route_tests.rs` holds it to a GET route.
pub(crate) const COOKIE_ONLY_GET: &[&str] = &[];

/// What the running server accepts: set once the web listener is bound.
struct Shape {
    cookie_name: String,
    /// Exactly the origins a page may have: `http://127.0.0.1:<port>`, and the declared one.
    origins: Vec<String>,
}

impl Shape {
    fn new(port: u16, instance_id: &str, declared: Option<&str>) -> Shape {
        let id: String = instance_id.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        let mut origins = vec![format!("http://127.0.0.1:{port}")];
        origins.extend(declared.map(str::to_string).filter(|o| *o != origins[0]));
        Shape { cookie_name: format!("{COOKIE_PREFIX}{id}"), origins }
    }

    fn host_ok(&self, host: &str) -> bool {
        self.origins.iter().any(|o| o.strip_prefix("http://") == Some(host))
    }
}

static SHAPE: OnceLock<Shape> = OnceLock::new();
static STORE: OnceLock<SessionStore> = OnceLock::new();

fn store() -> &'static SessionStore {
    STORE.get_or_init(SessionStore::default)
}

/// Turn the sign-in on, once the web listener has its port. Only for a headless srv that serves a UI.
pub(crate) fn enable(port: u16, instance_id: &str) {
    if crate::headless::local_sign_in() {
        let _ = SHAPE.set(Shape::new(port, instance_id, crate::headless::public_origin()));
    }
}

pub(crate) fn enabled() -> bool {
    SHAPE.get().is_some()
}

/// The `/auth/*` routes.
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/code", post(handle_code))
        .route("/auth/pending", post(handle_pending))
        .route("/auth/confirm", post(handle_confirm))
        .route("/auth/logout", post(handle_logout))
        .route("/auth/enter", get(handle_page).post(handle_enter))
        .route("/auth/enter.js", get(handle_script))
        .route("/auth/enter.css", get(handle_style))
        .route("/auth/wait", post(handle_wait))
        .layer(DefaultBodyLimit::max(1024))
}

// ---- Who may ask ----

/// Why a request was refused. For the log; the answer never says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    Code,
    Host,
    Origin,
    FetchSite,
    Key,
}

impl Reason {
    fn name(self) -> &'static str {
        match self {
            Reason::Code => "code",
            Reason::Host => "host",
            Reason::Origin => "origin",
            Reason::FetchSite => "fetch-site",
            Reason::Key => "key",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `agentmux-srv open`: no browser headers at all.
    Client,
    /// The sign-in page's own requests: same-origin.
    Browser,
    /// A page load or a session's request: only Host and Origin are held to.
    Any,
}

fn hval<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// `Host` is exactly an allowed one, `Origin` (where sent) is exactly an allowed origin, and
/// `Sec-Fetch-Site` suits the kind. A missing or unreadable header counts as wrong.
fn check_shape(shape: &Shape, headers: &HeaderMap, kind: Kind) -> Result<(), Reason> {
    if !hval(headers, "host").is_some_and(|h| shape.host_ok(h)) {
        return Err(Reason::Host);
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        if !origin.to_str().is_ok_and(|o| shape.origins.iter().any(|x| x == o)) {
            return Err(Reason::Origin);
        }
    }
    let site = headers.get("sec-fetch-site");
    let site_ok = match kind {
        Kind::Client => site.is_none(),
        Kind::Browser => site.is_some_and(|s| s == "same-origin"),
        Kind::Any => true,
    };
    if site_ok { Ok(()) } else { Err(Reason::FetchSite) }
}

/// The same 403 for every refusal, in the same time: the wait is the same for every reason.
async fn refuse(route: &'static str, reason: Reason) -> Response {
    static DELAYED: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(MAX_DELAYED);
    let recent = store().note_failure(Instant::now());
    match recent {
        n if n <= LOGGED_FAILURES => tracing::warn!(route, reason = reason.name(), failures_last_minute = n, "local sign-in refused"),
        n if n == LOGGED_FAILURES + 1 => tracing::warn!(failures_last_minute = n, "local sign-in refused again and again: more this minute are counted, not logged"),
        _ => {}
    }
    if let Ok(_slot) = DELAYED.try_acquire() {
        tokio::time::sleep(FAILURE_DELAY).await;
    }
    refusal()
}

fn refusal() -> Response {
    let mut r = (StatusCode::FORBIDDEN, "not accepted").into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn no_store(mut r: Response) -> Response {
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// `open` and the other commands: the shape of a non-browser caller, and the key.
async fn client_gate(state: &AppState, headers: &HeaderMap, route: &'static str) -> Result<(), Response> {
    let Some(shape) = SHAPE.get() else { return Err(refusal()) };
    if let Err(reason) = check_shape(shape, headers, Kind::Client) {
        return Err(refuse(route, reason).await);
    }
    let key_ok = hval(headers, AUTH_KEY_HEADER).is_some_and(|k| secret_eq(k.as_bytes(), state.auth_key.as_bytes()));
    if key_ok { Ok(()) } else { Err(refuse(route, Reason::Key).await) }
}

#[derive(Deserialize)]
struct CodeBody {
    code: String,
}

#[derive(Deserialize)]
struct ConfirmBody {
    code: String,
    accept: bool,
}

#[derive(Deserialize)]
struct HandleBody {
    handle: String,
}

// ---- The commands' routes (the key) ----

/// `POST /auth/code`: the key for a new one-time code.
async fn handle_code(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = client_gate(&state, &headers, "/auth/code").await {
        return r;
    }
    no_store(Json(json!({ "code": store().mint_code(Instant::now()) })).into_response())
}

/// `POST /auth/pending {code}`: what became of the code `open` minted.
async fn handle_pending(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = client_gate(&state, &headers, "/auth/pending").await {
        return r;
    }
    let Ok(CodeBody { code }) = serde_json::from_slice(&body) else { return refuse("/auth/pending", Reason::Code).await };
    let state = match store().code_state(&code, Instant::now()) {
        CodeState::Unused => json!({ "state": "unused" }),
        CodeState::Entered { phrase } => json!({ "state": "entered", "phrase": phrase }),
        CodeState::Gone => json!({ "state": "gone" }),
    };
    no_store(Json(state).into_response())
}

/// `POST /auth/confirm {code, accept}`: the person's answer for the session that code made.
async fn handle_confirm(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = client_gate(&state, &headers, "/auth/confirm").await {
        return r;
    }
    let Ok(ConfirmBody { code, accept }) = serde_json::from_slice(&body) else { return refuse("/auth/confirm", Reason::Code).await };
    let found = store().decide(&code, accept, Instant::now());
    tracing::info!(accepted = accept && found, found, "local sign-in answered");
    no_store(Json(json!({ "found": found })).into_response())
}

/// `POST /auth/logout`: end every browser session.
async fn handle_logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = client_gate(&state, &headers, "/auth/logout").await {
        return r;
    }
    let revoked = store().revoke_all();
    tracing::info!(revoked, "local sign-in sessions revoked");
    no_store(Json(json!({ "revoked": revoked })).into_response())
}

// ---- The browser's routes (the code) ----

/// `GET /auth/enter`: the page. It holds no secret; the code is in the fragment, which is never sent.
async fn handle_page(headers: HeaderMap) -> Response {
    let Some(shape) = SHAPE.get() else { return refusal() };
    if let Err(reason) = check_shape(shape, &headers, Kind::Any) {
        return refuse("/auth/enter", reason).await;
    }
    with_ui_headers(shape, no_store(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], ENTER_HTML).into_response()))
}

async fn handle_script() -> Response {
    let Some(shape) = SHAPE.get() else { return refusal() };
    with_ui_headers(shape, no_store(([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], ENTER_JS).into_response()))
}

async fn handle_style() -> Response {
    let Some(shape) = SHAPE.get() else { return refusal() };
    with_ui_headers(shape, no_store(([(header::CONTENT_TYPE, "text/css; charset=utf-8")], ENTER_CSS).into_response()))
}

/// `POST /auth/enter {code}`: spends the code and makes a pending session; the answer is the
/// handle to wait on and the check phrase to show. The session has no access yet.
async fn handle_enter(headers: HeaderMap, body: Bytes) -> Response {
    let Some(shape) = SHAPE.get() else { return refusal() };
    if let Err(reason) = check_shape(shape, &headers, Kind::Browser) {
        return refuse("/auth/enter", reason).await;
    }
    let entered = serde_json::from_slice::<CodeBody>(&body).ok().and_then(|b| store().enter(&b.code, Instant::now()));
    match entered {
        Some(e) => no_store(Json(json!({ "handle": e.handle, "phrase": e.phrase })).into_response()),
        None => refuse("/auth/enter", Reason::Code).await,
    }
}

/// `POST /auth/wait {handle}`: held until the person answers in the terminal. A confirmed session
/// is handed over once, here: the cookie in `Set-Cookie` and the token in the body.
async fn handle_wait(headers: HeaderMap, body: Bytes) -> Response {
    let Some(shape) = SHAPE.get() else { return refusal() };
    if let Err(reason) = check_shape(shape, &headers, Kind::Browser) {
        return refuse("/auth/wait", reason).await;
    }
    let Ok(HandleBody { handle }) = serde_json::from_slice(&body) else { return refuse("/auth/wait", Reason::Code).await };
    let until = Instant::now() + WAIT_HOLD;
    loop {
        match store().collect(&handle, Instant::now()) {
            Collected::Granted(grant) => {
                let cookie = format!("{}={}; HttpOnly; SameSite=Strict; Path=/", shape.cookie_name, grant.cookie);
                let mut r = no_store(Json(json!({ "token": grant.token })).into_response());
                if let Ok(v) = HeaderValue::from_str(&cookie) {
                    r.headers_mut().insert(header::SET_COOKIE, v);
                }
                return r;
            }
            Collected::Refused => return refuse("/auth/wait", Reason::Code).await,
            Collected::Waiting if Instant::now() >= until => return no_store((StatusCode::ACCEPTED, Json(json!({ "waiting": true }))).into_response()),
            Collected::Waiting => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    }
}

// ---- A browser's session on any other route ----

/// How a route takes a session. Every route needs the cookie and the token, except the GETs named
/// in [`COOKIE_ONLY_GET`].
pub(crate) fn route_class(method: &Method, path: &str) -> RouteClass {
    use crate::backend::route_audience::Match;
    let listed = *method == Method::GET && COOKIE_ONLY_GET.iter().any(|pattern| Match::Route(*pattern).matches(path));
    if listed { RouteClass::CookieOnly } else { RouteClass::Both }
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    hval(headers, "cookie")?.split(';').filter_map(|c| c.trim().split_once('=')).find(|(n, _)| *n == name).map(|(_, v)| v)
}

/// The token a request carries: `X-AM-Session`, or on `/ws` the protocol offered beside [`WS_PROTOCOL`].
fn session_token<'a>(headers: &'a HeaderMap, path: &str) -> Option<&'a str> {
    if let Some(t) = hval(headers, TOKEN_HEADER) {
        return Some(t);
    }
    if path != "/ws" {
        return None;
    }
    hval(headers, "sec-websocket-protocol")?.split(',').map(str::trim).find(|p| !p.is_empty() && *p != WS_PROTOCOL)
}

/// Does this request come from a live browser session? `Host` and `Origin` exact, and the cookie
/// and token (or the cookie alone where listed, with `Sec-Fetch-Site: same-origin`).
pub(crate) fn admit_session(req: &Request<Body>) -> Option<SessionId> {
    let shape = SHAPE.get()?;
    let headers = req.headers();
    check_shape(shape, headers, Kind::Any).ok()?;
    let class = route_class(req.method(), req.uri().path());
    if class == RouteClass::CookieOnly && hval(headers, "sec-fetch-site") != Some("same-origin") {
        return None;
    }
    let cookie = cookie_value(headers, &shape.cookie_name)?;
    let token = session_token(headers, req.uri().path());
    store().admit(Some(cookie), token, class, Instant::now()).then(|| SessionId::of_cookie(cookie))
}

/// Completes when the browser session behind a WebSocket ends; never for a connection that came
/// with the key.
pub(crate) async fn session_ended(session: Option<SessionId>) {
    match session {
        Some(id) => store().ended(id).await,
        None => std::future::pending().await,
    }
}

// ---- The served UI ----

/// The policy of the served UI and the sign-in page. `'unsafe-inline'` for styles only: the
/// terminal and layout set inline styles.
fn content_security_policy(shape: &Shape) -> String {
    let ws: Vec<String> = shape.origins.iter().map(|o| o.replacen("http://", "ws://", 1)).collect();
    format!(
        "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
         font-src 'self' data:; connect-src 'self' {}; worker-src 'self' blob:; object-src 'none'; base-uri 'none'; \
         form-action 'self'; frame-ancestors 'none'",
        ws.join(" ")
    )
}

fn with_ui_headers(shape: &Shape, mut r: Response) -> Response {
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&content_security_policy(shape)) {
        h.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    r
}

/// Middleware for the served UI's files.
pub(crate) async fn ui_headers(req: Request<Body>, next: Next) -> Response {
    let response = next.run(req).await;
    match SHAPE.get() {
        Some(shape) => with_ui_headers(shape, response),
        None => response,
    }
}

const ENTER_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<title>AgentMux sign-in</title>
<link rel="stylesheet" href="/auth/enter.css">
</head>
<body>
<main>
<h1>AgentMux</h1>
<p id="msg">Signing in&hellip;</p>
<p id="phrase" hidden></p>
<p id="hint" hidden>Compare this with the terminal where you ran <code>agentmux-srv open</code>. Answer <code>y</code> there only if the two match.</p>
</main>
<script src="/auth/enter.js"></script>
</body>
</html>
"#;

const ENTER_CSS: &str = "body{font-family:system-ui,sans-serif;background:#111;color:#eee;display:flex;justify-content:center;padding-top:15vh}\
main{max-width:34rem;padding:0 1rem;text-align:center}\
#phrase{font-family:ui-monospace,monospace;font-size:3rem;letter-spacing:.05em;margin:1.5rem 0}\
#hint{color:#bbb}code{background:#222;padding:.1rem .3rem;border-radius:.2rem}\n";

/// The code is in the fragment. The history entry loses it before anything is sent; the page then
/// waits for the person's answer in the terminal and keeps the token in `sessionStorage` only.
const ENTER_JS: &str = r#"(async () => {
  const msg = document.getElementById("msg");
  const phrase = document.getElementById("phrase");
  const hint = document.getElementById("hint");
  const fail = () => {
    msg.textContent = "not accepted";
    phrase.hidden = true;
    hint.hidden = true;
  };
  const code = location.hash.slice(1);
  history.replaceState(null, "", location.pathname);
  if (!code) return fail();
  const post = (path, body) =>
    fetch(path, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      cache: "no-store",
      credentials: "same-origin",
    });
  let entered;
  try {
    const r = await post("/auth/enter", { code });
    if (!r.ok) return fail();
    entered = await r.json();
  } catch {
    return fail();
  }
  msg.textContent = "Waiting for you to confirm in the terminal.";
  phrase.textContent = entered.phrase;
  phrase.hidden = false;
  hint.hidden = false;
  for (;;) {
    let w;
    try {
      w = await post("/auth/wait", { handle: entered.handle });
    } catch {
      return fail();
    }
    if (w.status === 202) continue;
    if (!w.ok) return fail();
    const { token } = await w.json();
    sessionStorage.setItem("amx-session", token);
    location.replace("/");
    return;
  }
})();
"#;

#[cfg(test)]
#[path = "tests/local_session_route_tests.rs"]
mod tests;
