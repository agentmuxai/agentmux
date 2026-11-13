// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::AUTH_KEY_HEADER;
use axum::http::{HeaderMap, Request};
use tower::ServiceExt;

use super::*;
use crate::server::local_proof::{self, Client, HelloReply};
use crate::server::route_scan::registered_routes;
use crate::server::routes::build_router;
use crate::server::tests::test_state;

const HOST: &str = "127.0.0.1:7777";
const KEY: &str = "test-secret-key";
const COOKIE: &str = "amx-s-test-instance";

/// The store is the running server's, shared by every test of this file; a logout ends every session in it.
/// A test that signs a browser in and then relies on it holds this for its whole length.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The shape every test shares (a process-wide one-time setting, like the running server's).
fn ensure_shape() -> &'static Shape {
    let _ = SHAPE.set(Shape::new(7777, "test-instance", Some("http://127.0.0.1:8080")));
    SHAPE.get().unwrap()
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        h.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
    }
    h
}

fn request(method: Method, uri: &str, hs: &[(&str, &str)], body: &str) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    for (k, v) in hs {
        b = b.header(*k, *v);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

async fn call(router: &Router, req: Request<Body>) -> (u16, HeaderMap, String) {
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status().as_u16();
    let hs = resp.headers().clone();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (status, hs, String::from_utf8_lossy(&body).into_owned())
}

fn field(body: &str, name: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(body).unwrap()[name].clone() {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    }
}

/// `open`'s end of a channel, begun through the router the way `open` begins it.
struct Open {
    client: Client,
}

impl Open {
    async fn connect(router: &Router) -> Open {
        let nonce = local_proof::new_nonce();
        let hello = request(Method::POST, "/auth/hello", &[("host", HOST)], &format!(r#"{{"nonce":"{nonce}"}}"#));
        let (status, _, body) = call(router, hello).await;
        assert_eq!(status, 200, "{body}");
        let reply = HelloReply { channel: field(&body, "channel"), nonce: field(&body, "nonce"), declared: field(&body, "declared"), proof: field(&body, "proof") };
        Open { client: Client::accept(KEY.as_bytes(), &nonce, HOST, reply).expect("srv proves it holds the key at its own address") }
    }

    /// A command signed on this channel.
    fn command(&mut self, path: &str, body: &str) -> (Request<Body>, u64) {
        let signed = self.client.sign("POST", path, body.as_bytes());
        let req = request(
            Method::POST,
            path,
            &[("host", HOST), (local_proof::CHANNEL_HEADER, &signed.channel), (local_proof::SEQ_HEADER, &signed.seq.to_string()), (local_proof::PROOF_HEADER, &signed.proof)],
            body,
        );
        (req, signed.seq)
    }

    /// Send a command and read the answer, which must carry the channel's proof; returns the body.
    async fn ask(&mut self, router: &Router, path: &str, body: &str) -> String {
        let (req, seq) = self.command(path, body);
        let (status, hs, answer) = call(router, req).await;
        assert_eq!(status, 200, "{path}: {answer}");
        let proof = hs.get(local_proof::PROOF_HEADER).expect("the answer carries a proof").to_str().unwrap();
        assert!(self.client.answer_ok(seq, status, answer.as_bytes(), proof), "{path}: the answer's proof");
        assert!(!self.client.answer_ok(seq, status, format!("{answer} ").as_bytes(), proof), "{path}: a changed answer");
        answer
    }
}

fn from_page(path: &str, body: &str) -> Request<Body> {
    request(Method::POST, path, &[("host", HOST), ("sec-fetch-site", "same-origin")], body)
}

// ---- Who may ask ----

#[test]
fn only_the_one_host_name_and_the_declared_origin_are_accepted() {
    let shape = ensure_shape();
    let ok = |hs: &[(&str, &str)], kind| check_shape(shape, &headers(hs), kind);
    assert_eq!(ok(&[("host", HOST)], Kind::Client), Ok(()));
    assert_eq!(ok(&[("host", "127.0.0.1:8080")], Kind::Client), Ok(()), "the declared origin's host");
    for bad in ["localhost:7777", "127.0.0.1:7778", "127.0.0.1", "127.0.0.1:7777.evil.test", "[::1]:7777", "evil.test"] {
        assert_eq!(ok(&[("host", bad)], Kind::Any), Err(Reason::Host), "{bad}");
    }
    assert_eq!(check_shape(shape, &HeaderMap::new(), Kind::Any), Err(Reason::Host), "no Host at all");
    // Origin, where sent, is exactly an allowed one.
    assert_eq!(ok(&[("host", HOST), ("origin", "http://127.0.0.1:7777")], Kind::Any), Ok(()));
    for bad in ["http://localhost:7777", "http://127.0.0.1:7778", "null", "https://127.0.0.1:7777", "http://127.0.0.1:7777/"] {
        assert_eq!(ok(&[("host", HOST), ("origin", bad)], Kind::Any), Err(Reason::Origin), "{bad}");
    }
}

#[test]
fn a_command_sends_no_browser_headers_and_the_page_sends_same_origin() {
    let shape = ensure_shape();
    let with = |site: Option<&str>, kind| {
        let mut hs = vec![("host", HOST)];
        hs.extend(site.map(|s| ("sec-fetch-site", s)));
        check_shape(shape, &headers(&hs), kind)
    };
    assert_eq!(with(None, Kind::Client), Ok(()));
    for site in ["same-origin", "none", "cross-site", "same-site"] {
        assert_eq!(with(Some(site), Kind::Client), Err(Reason::FetchSite), "a command carrying {site}");
    }
    assert_eq!(with(Some("same-origin"), Kind::Browser), Ok(()));
    for site in [None, Some("none"), Some("cross-site"), Some("same-site")] {
        assert_eq!(with(site, Kind::Browser), Err(Reason::FetchSite), "a post from {site:?}");
    }
    assert_eq!(with(Some("none"), Kind::Any), Ok(()), "the page itself opens from outside as `none`");
}

/// Every kind of failure of a sign-in route gives the same answer.
#[tokio::test]
async fn every_failure_gives_the_same_answer() {
    ensure_shape();
    let router = build_router(test_state());
    let mut open = Open::connect(&router).await;
    let signed = |open: &mut Open, path: &str, extra: &[(&str, &str)]| {
        let s = open.client.sign("POST", path, b"{}");
        let mut hs = vec![(local_proof::CHANNEL_HEADER.to_string(), s.channel), (local_proof::SEQ_HEADER.to_string(), s.seq.to_string()), (local_proof::PROOF_HEADER.to_string(), s.proof)];
        hs.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let hs: Vec<(&str, &str)> = hs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        request(Method::POST, path, &hs, "{}")
    };
    let evil_origin = [("host", HOST), ("origin", "http://evil.test"), ("sec-fetch-site", "same-origin")];
    let cases: Vec<(&str, Request<Body>)> = vec![
        ("code, the key itself", request(Method::POST, "/auth/code", &[("host", HOST), (AUTH_KEY_HEADER, KEY)], "{}")),
        ("code, no proof", request(Method::POST, "/auth/code", &[("host", HOST)], "{}")),
        ("code, unknown channel", request(Method::POST, "/auth/code", &[("host", HOST), (local_proof::CHANNEL_HEADER, &"a".repeat(64)), (local_proof::SEQ_HEADER, "1"), (local_proof::PROOF_HEADER, &"0".repeat(64))], "{}")),
        ("code, wrong proof", request(Method::POST, "/auth/code", &[("host", HOST), (local_proof::CHANNEL_HEADER, &open.client.sign("POST", "/auth/code", b"{}").channel), (local_proof::SEQ_HEADER, "9"), (local_proof::PROOF_HEADER, &"0".repeat(64))], "{}")),
        ("code, other host", signed(&mut open, "/auth/code", &[("host", "localhost:7777")])),
        ("code, origin", signed(&mut open, "/auth/code", &[("host", HOST), ("origin", "http://evil.test")])),
        ("code, browser header", signed(&mut open, "/auth/code", &[("host", HOST), ("sec-fetch-site", "same-origin")])),
        ("hello, other host", request(Method::POST, "/auth/hello", &[("host", "localhost:7777")], &format!(r#"{{"nonce":"{}"}}"#, local_proof::new_nonce()))),
        ("hello, not a nonce", request(Method::POST, "/auth/hello", &[("host", HOST)], r#"{"nonce":"abc"}"#)),
        ("hello, not json", request(Method::POST, "/auth/hello", &[("host", HOST)], "nope")),
        ("enter, wrong code", from_page("/auth/enter", r#"{"code":"nope"}"#)),
        ("enter, not json", from_page("/auth/enter", "nope")),
        ("enter, other host", request(Method::POST, "/auth/enter", &[("host", "evil.test"), ("sec-fetch-site", "same-origin")], r#"{"code":"x"}"#)),
        ("enter, origin", request(Method::POST, "/auth/enter", &evil_origin, r#"{"code":"x"}"#)),
        ("enter, cross-site", request(Method::POST, "/auth/enter", &[("host", HOST), ("sec-fetch-site", "cross-site")], r#"{"code":"x"}"#)),
        ("enter, no fetch metadata", request(Method::POST, "/auth/enter", &[("host", HOST)], r#"{"code":"x"}"#)),
        ("wait, unknown handle", from_page("/auth/wait", r#"{"handle":"nope"}"#)),
    ];
    let mut answers = Vec::new();
    for (name, req) in cases {
        let (status, hs, body) = call(&router, req).await;
        // The CORS layer varies by the request's Origin; everything srv's sign-in sets must match.
        let mut ours: Vec<String> = hs
            .iter()
            .filter(|(k, _)| !k.as_str().starts_with("access-control-") && k.as_str() != "vary")
            .map(|(k, v)| format!("{k}: {}", v.to_str().unwrap()))
            .collect();
        ours.sort();
        answers.push((name, status, ours, body));
    }
    let (first_name, first_status, first_headers, first_body) = answers[0].clone();
    assert_eq!((first_status, first_body.as_str()), (403, "not accepted"), "{first_name}");
    for (name, status, hs, body) in &answers[1..] {
        assert_eq!((status, hs, body), (&first_status, &first_headers, &first_body), "{name} differs from {first_name}");
    }
    assert!(first_headers.iter().any(|h| h == "cache-control: no-store"), "{first_headers:?}");
}

#[tokio::test(start_paused = true)]
async fn a_refusal_waits_a_fixed_quarter_second_whatever_was_wrong() {
    ensure_shape();
    for reason in [Reason::Code, Reason::Host, Reason::Origin, Reason::FetchSite, Reason::Proof] {
        let before = tokio::time::Instant::now();
        let r = refuse("/auth/test", reason).await;
        assert_eq!(r.status().as_u16(), 403);
        assert_eq!(tokio::time::Instant::now() - before, FAILURE_DELAY, "{reason:?}");
    }
}

// ---- The whole sign-in, through the router ----

#[tokio::test]
async fn a_browser_signs_in_with_a_code_the_person_confirms_and_only_then_reaches_the_api() {
    let _only = ONE_AT_A_TIME.lock().await;
    ensure_shape();
    let router = build_router(test_state());
    let probe = |hs: Vec<(&str, String)>| {
        let hs: Vec<(&str, &str)> = hs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        request(Method::GET, "/agentmux/work/boards", &hs, "")
    };

    // `open` mints a code on a channel srv proved itself on.
    let mut open = Open::connect(&router).await;
    let body = open.ask(&router, "/auth/code", "{}").await;
    let code = field(&body, "code");
    assert_eq!(code.len(), 64);
    let pending_body = format!(r#"{{"code":"{code}"}}"#);
    assert_eq!(field(&open.ask(&router, "/auth/pending", &pending_body).await, "state"), "unused");

    // The browser enters it and holds a pending session with a phrase.
    let (status, _, body) = call(&router, from_page("/auth/enter", &format!(r#"{{"code":"{code}"}}"#))).await;
    assert_eq!(status, 200);
    let (handle, phrase) = (field(&body, "handle"), field(&body, "phrase"));
    let body = open.ask(&router, "/auth/pending", &pending_body).await;
    assert_eq!((field(&body, "state"), field(&body, "phrase")), ("entered".to_string(), phrase.clone()));

    // A second entry of the same code is refused.
    assert_eq!(call(&router, from_page("/auth/enter", &format!(r#"{{"code":"{code}"}}"#))).await.0, 403);

    // The pending session reaches nothing: its handle is no cookie and no token.
    let nothing = probe(vec![("host", HOST.into()), ("cookie", format!("{COOKIE}={handle}")), (TOKEN_HEADER, handle.clone())]);
    assert_eq!(call(&router, nothing).await.0, 401);

    // The page waits; the person confirms in the terminal.
    let waiting = tokio::spawn({
        let router = router.clone();
        let body = format!(r#"{{"handle":"{handle}"}}"#);
        async move { call(&router, from_page("/auth/wait", &body)).await }
    });
    tokio::task::yield_now().await;
    let confirmed = open.ask(&router, "/auth/confirm", &format!(r#"{{"code":"{code}","accept":true}}"#)).await;
    assert_eq!(field(&confirmed, "found"), "true");
    let (status, hs, body) = waiting.await.unwrap();
    assert_eq!(status, 200);
    let token = field(&body, "token");
    let set_cookie = hs.get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
    assert!(set_cookie.starts_with(&format!("{COOKIE}=")), "{set_cookie}");
    assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict") && set_cookie.contains("Path=/"));
    let cookie = set_cookie.split(';').next().unwrap().to_string();

    // A second wait with the spent handle is refused.
    assert_eq!(call(&router, from_page("/auth/wait", &format!(r#"{{"handle":"{handle}"}}"#))).await.0, 403);

    // The pair reaches the API; a cookie alone, a token alone, another host or origin do not.
    let both = |extra: Vec<(&'static str, String)>| {
        let mut hs = vec![("host", HOST.to_string()), ("cookie", cookie.clone()), (TOKEN_HEADER, token.clone())];
        hs.extend(extra);
        probe(hs)
    };
    assert_ne!(call(&router, both(vec![])).await.0, 401);
    assert_ne!(call(&router, both(vec![("origin", "http://127.0.0.1:7777".into())])).await.0, 401);
    assert_eq!(call(&router, probe(vec![("host", HOST.into()), ("cookie", cookie.clone())])).await.0, 401, "a cookie alone");
    assert_eq!(call(&router, probe(vec![("host", HOST.into()), (TOKEN_HEADER, token.clone())])).await.0, 401, "a token alone");
    assert_eq!(call(&router, probe(vec![("host", "localhost:7777".into()), ("cookie", cookie.clone()), (TOKEN_HEADER, token.clone())])).await.0, 401);
    assert_eq!(call(&router, both(vec![("origin", "http://evil.test".into())])).await.0, 401);

    // Logging out ends it.
    let body = open.ask(&router, "/auth/logout", "{}").await;
    assert!(serde_json::from_str::<serde_json::Value>(&body).unwrap()["revoked"].as_u64().unwrap() >= 1);
    assert_eq!(call(&router, both(vec![])).await.0, 401);
}

#[tokio::test]
async fn a_discarded_sign_in_gives_the_browser_nothing_and_the_person_a_line() {
    ensure_shape();
    let router = build_router(test_state());
    let mut open = Open::connect(&router).await;
    let code = field(&open.ask(&router, "/auth/code", "{}").await, "code");
    let entered = call(&router, from_page("/auth/enter", &format!(r#"{{"code":"{code}"}}"#))).await.2;
    let handle = field(&entered, "handle");
    let no = open.ask(&router, "/auth/confirm", &format!(r#"{{"code":"{code}","accept":false}}"#)).await;
    assert_eq!(field(&no, "found"), "true");
    let (status, hs, _) = call(&router, from_page("/auth/wait", &format!(r#"{{"handle":"{handle}"}}"#))).await;
    assert_eq!(status, 403);
    assert!(hs.get(header::SET_COOKIE).is_none());
    // Nobody else's code is answerable.
    let other = open.ask(&router, "/auth/confirm", r#"{"code":"someone else's","accept":true}"#).await;
    assert_eq!(field(&other, "found"), "false");
}

// ---- Signing out one browser ----

/// A browser signed in the whole way, through the router: (its cookie pair, its token).
async fn signed_in_browser(router: &Router) -> (String, String) {
    let mut open = Open::connect(router).await;
    let code = field(&open.ask(router, "/auth/code", "{}").await, "code");
    let handle = field(&call(router, from_page("/auth/enter", &format!(r#"{{"code":"{code}"}}"#))).await.2, "handle");
    open.ask(router, "/auth/confirm", &format!(r#"{{"code":"{code}","accept":true}}"#)).await;
    let (status, hs, body) = call(router, from_page("/auth/wait", &format!(r#"{{"handle":"{handle}"}}"#))).await;
    assert_eq!(status, 200, "{body}");
    let cookie = hs.get(header::SET_COOKIE).unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
    (cookie, field(&body, "token"))
}

fn from_browser(method: Method, path: &str, cookie: &str, token: &str, extra: &[(&str, &str)]) -> Request<Body> {
    let mut hs = vec![("host", HOST), ("cookie", cookie), (TOKEN_HEADER, token)];
    hs.extend(extra.iter().copied());
    request(method, path, &hs, "")
}

#[tokio::test]
async fn a_browser_signs_itself_out_and_no_other_browser_with_it() {
    let _only = ONE_AT_A_TIME.lock().await;
    ensure_shape();
    let router = build_router(test_state());
    let (cookie_a, token_a) = signed_in_browser(&router).await;
    let (cookie_b, token_b) = signed_in_browser(&router).await;
    let probe = |cookie: &str, token: &str| from_browser(Method::GET, "/agentmux/work/boards", cookie, token, &[]);
    assert_ne!(call(&router, probe(&cookie_a, &token_a)).await.0, 401);

    // Not by the cookie alone, not by the token alone, not from another site, not by GET.
    let signout = |extra: &[(&str, &str)], cookie: &str, token: &str| from_browser(Method::POST, "/auth/signout", cookie, token, extra);
    for (name, req) in [
        ("no fetch metadata", signout(&[], &cookie_a, &token_a)),
        ("cross-site", signout(&[("sec-fetch-site", "cross-site")], &cookie_a, &token_a)),
        ("another browser's token", signout(&[("sec-fetch-site", "same-origin")], &cookie_a, &token_b)),
        ("a bad cookie", signout(&[("sec-fetch-site", "same-origin")], "amx-s-test-instance=nope", &token_a)),
    ] {
        assert_eq!(call(&router, req).await.0, 403, "{name}");
    }
    let cookie_only = request(Method::POST, "/auth/signout", &[("host", HOST), ("cookie", &cookie_a), ("sec-fetch-site", "same-origin")], "");
    assert_eq!(call(&router, cookie_only).await.0, 403, "cookie alone");
    assert_ne!(call(&router, probe(&cookie_a, &token_a)).await.0, 401, "nothing of that ended it");

    // From the page: only this browser's session ends, and the cookie is told to go.
    let (status, hs, body) = call(&router, signout(&[("sec-fetch-site", "same-origin")], &cookie_a, &token_a)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(field(&body, "signedOut"), "true");
    let set = hs.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(set.starts_with("amx-s-test-instance=;") && set.contains("Max-Age=0"), "{set}");
    assert_eq!(call(&router, probe(&cookie_a, &token_a)).await.0, 401, "this browser is out");
    assert_ne!(call(&router, probe(&cookie_b, &token_b)).await.0, 401, "the other browser stays");
    // A second sign-out of the same session is refused like any request without one.
    assert_eq!(call(&router, signout(&[("sec-fetch-site", "same-origin")], &cookie_a, &token_a)).await.0, 403);
}

// ---- The proof of the key ----

/// The key is never sent: where it used to be accepted, it is now refused, on every key route.
#[tokio::test]
async fn the_key_itself_is_not_accepted_on_any_command_route() {
    ensure_shape();
    let router = build_router(test_state());
    for path in ["/auth/code", "/auth/pending", "/auth/confirm", "/auth/logout"] {
        let req = request(Method::POST, path, &[("host", HOST), (AUTH_KEY_HEADER, KEY)], r#"{"code":"x","accept":true}"#);
        let (status, _, body) = call(&router, req).await;
        assert_eq!((status, body.as_str()), (403, "not accepted"), "{path}");
    }
}

#[tokio::test]
async fn a_signed_command_works_once_only_as_signed_and_only_on_its_channel() {
    ensure_shape();
    let router = build_router(test_state());
    let mut open = Open::connect(&router).await;
    let mut other = Open::connect(&router).await;

    // The same request again (a copy of it taken from the wire).
    let (req, _) = open.command("/auth/code", "{}");
    let replay = clone_request(&req);
    assert_eq!(call(&router, req).await.0, 200);
    assert_eq!(call(&router, replay).await.0, 403, "a replay");

    // Signed for one route, sent to another; a body changed after it was signed.
    let (pending, _) = open.command("/auth/pending", r#"{"code":"x"}"#);
    let (mut as_code, _) = open.command("/auth/pending", "{}");
    *as_code.uri_mut() = "/auth/code".parse().unwrap();
    assert_eq!(call(&router, as_code).await.0, 403, "another route");
    let (signed_for, _) = other.command("/auth/confirm", r#"{"code":"x","accept":false}"#);
    let (parts, _) = signed_for.into_parts();
    let changed = Request::from_parts(parts, Body::from(r#"{"code":"x","accept":true}"#));
    assert_eq!(call(&router, changed).await.0, 403, "another body");
    assert_eq!(call(&router, pending).await.0, 200, "the untouched one still works");

    // Another channel's proof does not do.
    let (mut from_other, _) = other.command("/auth/code", "{}");
    let first = open.command("/auth/code", "{}").0;
    from_other.headers_mut().insert(local_proof::CHANNEL_HEADER, first.headers()[local_proof::CHANNEL_HEADER].clone());
    assert_eq!(call(&router, from_other).await.0, 403, "a channel's number and proof under another channel");
}

/// srv's proof is for the address it is bound to: a forwarder on another port gets the same answer
/// and `open`, talking to its own port, does not accept it.
#[tokio::test]
async fn srv_proves_the_address_it_is_bound_to() {
    ensure_shape();
    let router = build_router(test_state());
    let nonce = local_proof::new_nonce();
    let hello = request(Method::POST, "/auth/hello", &[("host", HOST)], &format!(r#"{{"nonce":"{nonce}"}}"#));
    let body = call(&router, hello).await.2;
    let reply = |declared: String| HelloReply { channel: field(&body, "channel"), nonce: field(&body, "nonce"), declared, proof: field(&body, "proof") };
    assert!(Client::accept(KEY.as_bytes(), &nonce, HOST, reply(field(&body, "declared"))).is_ok());
    assert_eq!(field(&body, "declared"), "http://127.0.0.1:8080");
    assert!(Client::accept(KEY.as_bytes(), &nonce, "127.0.0.1:7778", reply(field(&body, "declared"))).is_err(), "a forwarder's port");
    assert!(Client::accept(b"another-key", &nonce, HOST, reply(field(&body, "declared"))).is_err(), "a listener without the key");
}

fn clone_request(req: &Request<Body>) -> Request<Body> {
    let mut b = Request::builder().method(req.method().clone()).uri(req.uri().clone());
    for (k, v) in req.headers() {
        b = b.header(k, v);
    }
    b.body(Body::from("{}")).unwrap()
}

// ---- Routes and how a request carries its session ----

#[test]
fn the_token_is_a_header_or_on_the_websocket_the_protocol_beside_the_fixed_name() {
    let hs = headers(&[(TOKEN_HEADER, "abc")]);
    assert_eq!(session_token(&hs, "/agentmux/service"), Some("abc"));
    let ws = headers(&[("sec-websocket-protocol", "amx-session, tok123")]);
    assert_eq!(session_token(&ws, "/ws"), Some("tok123"));
    assert_eq!(session_token(&headers(&[("sec-websocket-protocol", "tok123, amx-session")]), "/ws"), Some("tok123"));
    assert_eq!(session_token(&ws, "/agentmux/service"), None, "a protocol header is a token on /ws only");
    assert_eq!(session_token(&headers(&[("sec-websocket-protocol", "amx-session")]), "/ws"), None);
    assert_eq!(session_token(&HeaderMap::new(), "/ws"), None);
}

#[test]
fn the_cookie_is_found_by_its_name_among_others() {
    let hs = headers(&[("cookie", "a=1; amx-s-test-instance=zzz; amx-s-other=yyy")]);
    assert_eq!(cookie_value(&hs, "amx-s-test-instance"), Some("zzz"));
    assert_eq!(cookie_value(&hs, "amx-s-other"), Some("yyy"));
    assert_eq!(cookie_value(&hs, "amx-s-missing"), None);
    assert_eq!(cookie_value(&HeaderMap::new(), "x"), None);
}

#[test]
fn the_cookie_name_is_per_instance_and_safe() {
    assert_eq!(Shape::new(1, "default", None).cookie_name, "amx-s-default");
    assert_eq!(Shape::new(1, "a b;c=d", None).cookie_name, "amx-s-a-b-c-d");
}

/// Each route takes the session pair, except the GETs the list names, which must be GET-only routes.
#[test]
fn every_route_takes_both_the_cookie_and_the_token_unless_it_is_named_cookie_only() {
    let sample = |p: &str| p.split('/').map(|s| if s.starts_with(':') || s.starts_with('*') { "x" } else { s }).collect::<Vec<_>>().join("/");
    let routes = registered_routes();
    assert!(routes.len() > 100, "the scan found the route table");
    for route in &routes {
        for (name, method) in [("get", Method::GET), ("post", Method::POST), ("put", Method::PUT), ("patch", Method::PATCH), ("delete", Method::DELETE)] {
            if !route.methods.contains(&format!("{name}(")) {
                continue;
            }
            let class = route_class(&method, &sample(&route.path));
            let named = method == Method::GET && COOKIE_ONLY_GET.contains(&route.path.as_str());
            let want = if named { RouteClass::CookieOnly } else { RouteClass::Both };
            assert_eq!(class, want, "{method} {}", route.path);
        }
    }
    for listed in COOKIE_ONLY_GET {
        let route = routes.iter().find(|r| r.path == *listed).unwrap_or_else(|| panic!("{listed} is not a registered route"));
        assert!(route.methods.contains("get("), "{listed} is not a GET");
        for other in ["post(", "put(", "patch(", "delete("] {
            assert!(!route.methods.contains(other), "{listed} also takes {other}");
        }
    }
}

#[test]
fn a_cookie_only_get_needs_same_origin_fetch_metadata() {
    ensure_shape();
    let req = |site: Option<&str>| {
        let mut hs = vec![("host", HOST), ("cookie", "amx-s-test-instance=none")];
        hs.extend(site.map(|s| ("sec-fetch-site", s)));
        request(Method::GET, "/agentmux/work/boards", &hs, "")
    };
    // Not a listed route, so it needs the token as well: nothing here is admitted.
    for site in [None, Some("same-origin"), Some("none")] {
        assert!(admit_session(&req(site)).is_none());
    }
}

// ---- The served UI ----

#[test]
fn the_policy_is_closed_by_default_and_names_only_the_websocket_origins() {
    let shape = ensure_shape();
    let csp = content_security_policy(shape);
    for directive in [
        "default-src 'none'",
        "script-src 'self' 'wasm-unsafe-eval';",
        "object-src 'none'",
        "base-uri 'self';",
        "form-action 'self'",
        "frame-ancestors 'none'",
        "connect-src 'self' ws://127.0.0.1:7777 ws://127.0.0.1:8080;",
    ] {
        assert!(csp.contains(directive), "{directive} in {csp}");
    }
    // WebAssembly may compile (the code highlighter's engine) and nothing else of the kind is allowed.
    assert!(!csp.contains("script-src 'self' 'unsafe-inline'") && !csp.contains("'unsafe-eval'") && !csp.contains("'unsafe-hashes'"), "{csp}");
    assert_eq!(csp.matches("wasm-unsafe-eval").count(), 1, "{csp}");
    assert_eq!(csp.matches("'unsafe-inline'").count(), 1, "styles only");
}

#[tokio::test]
async fn the_sign_in_page_and_its_files_carry_the_policy_and_cannot_be_framed() {
    ensure_shape();
    let router = build_router(test_state());
    let page = request(Method::GET, "/auth/enter", &[("host", HOST), ("sec-fetch-site", "none")], "");
    let (status, hs, body) = call(&router, page).await;
    assert_eq!(status, 200);
    assert!(body.contains(r#"<script src="/auth/enter.js">"#) && !body.contains("<script>"), "no inline script");
    for (name, want) in [("content-security-policy", "frame-ancestors 'none'"), ("x-frame-options", "DENY"), ("cache-control", "no-store"), ("referrer-policy", "no-referrer")] {
        assert!(hs.get(name).unwrap().to_str().unwrap().contains(want), "{name}");
    }
    // The page does not come up for another host name.
    let other = request(Method::GET, "/auth/enter", &[("host", "localhost:7777")], "");
    assert_eq!(call(&router, other).await.0, 403);
    let (status, hs, js) = call(&router, request(Method::GET, "/auth/enter.js", &[("host", HOST)], "")).await;
    assert_eq!(status, 200);
    assert!(hs.get(header::CONTENT_TYPE).unwrap().to_str().unwrap().starts_with("text/javascript"));
    // The code never leaves the fragment: the entry is replaced before anything is posted.
    assert!(js.find("history.replaceState").unwrap() < js.find("post(\"/auth/enter\"").unwrap());
    assert!(js.contains("sessionStorage.setItem(\"amx-session\"") && !js.contains("localStorage"));
}

#[test]
fn the_served_ui_files_get_the_same_headers() {
    let shape = ensure_shape();
    let r = with_ui_headers(shape, "x".into_response());
    assert!(r.headers().get(header::CONTENT_SECURITY_POLICY).is_some());
    assert_eq!(r.headers().get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
    assert_eq!(r.headers().get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
}
