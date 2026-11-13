// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use axum::http::{HeaderMap, Request};
use tower::ServiceExt;

use super::*;
use crate::server::route_scan::registered_routes;
use crate::server::routes::build_router;
use crate::server::tests::test_state;

const HOST: &str = "127.0.0.1:7777";
const KEY: &str = "test-secret-key";
const COOKIE: &str = "amx-s-test-instance";

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

fn from_command(path: &str, body: &str) -> Request<Body> {
    request(Method::POST, path, &[("host", HOST), (AUTH_KEY_HEADER, KEY)], body)
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
    let evil_origin = [("host", HOST), ("origin", "http://evil.test"), ("sec-fetch-site", "same-origin")];
    let cases: Vec<(&str, Request<Body>)> = vec![
        ("code, wrong key", request(Method::POST, "/auth/code", &[("host", HOST), (AUTH_KEY_HEADER, "wrong")], "")),
        ("code, no key", request(Method::POST, "/auth/code", &[("host", HOST)], "")),
        ("code, other host", request(Method::POST, "/auth/code", &[("host", "localhost:7777"), (AUTH_KEY_HEADER, KEY)], "")),
        ("code, origin", request(Method::POST, "/auth/code", &[("host", HOST), ("origin", "http://evil.test"), (AUTH_KEY_HEADER, KEY)], "")),
        ("code, browser header", request(Method::POST, "/auth/code", &[("host", HOST), ("sec-fetch-site", "same-origin"), (AUTH_KEY_HEADER, KEY)], "")),
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
    for reason in [Reason::Code, Reason::Host, Reason::Origin, Reason::FetchSite, Reason::Key] {
        let before = tokio::time::Instant::now();
        let r = refuse("/auth/test", reason).await;
        assert_eq!(r.status().as_u16(), 403);
        assert_eq!(tokio::time::Instant::now() - before, FAILURE_DELAY, "{reason:?}");
    }
}

// ---- The whole sign-in, through the router ----

#[tokio::test]
async fn a_browser_signs_in_with_a_code_the_person_confirms_and_only_then_reaches_the_api() {
    ensure_shape();
    let router = build_router(test_state());
    let probe = |hs: Vec<(&str, String)>| {
        let hs: Vec<(&str, &str)> = hs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        request(Method::GET, "/agentmux/work/boards", &hs, "")
    };

    // `open` mints a code with the key.
    let (status, _, body) = call(&router, from_command("/auth/code", "")).await;
    assert_eq!(status, 200);
    let code = field(&body, "code");
    assert_eq!(code.len(), 64);
    let pending = |code: &str| from_command("/auth/pending", &format!(r#"{{"code":"{code}"}}"#));
    assert_eq!(field(&call(&router, pending(&code)).await.2, "state"), "unused");

    // The browser enters it and holds a pending session with a phrase.
    let (status, _, body) = call(&router, from_page("/auth/enter", &format!(r#"{{"code":"{code}"}}"#))).await;
    assert_eq!(status, 200);
    let (handle, phrase) = (field(&body, "handle"), field(&body, "phrase"));
    let (_, _, body) = call(&router, pending(&code)).await;
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
    let confirm = from_command("/auth/confirm", &format!(r#"{{"code":"{code}","accept":true}}"#));
    assert_eq!(field(&call(&router, confirm).await.2, "found"), "true");
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
    let (_, _, body) = call(&router, from_command("/auth/logout", "")).await;
    assert!(serde_json::from_str::<serde_json::Value>(&body).unwrap()["revoked"].as_u64().unwrap() >= 1);
    assert_eq!(call(&router, both(vec![])).await.0, 401);
}

#[tokio::test]
async fn a_discarded_sign_in_gives_the_browser_nothing_and_the_person_a_line() {
    ensure_shape();
    let router = build_router(test_state());
    let code = field(&call(&router, from_command("/auth/code", "")).await.2, "code");
    let entered = call(&router, from_page("/auth/enter", &format!(r#"{{"code":"{code}"}}"#))).await.2;
    let handle = field(&entered, "handle");
    let no = from_command("/auth/confirm", &format!(r#"{{"code":"{code}","accept":false}}"#));
    assert_eq!(field(&call(&router, no).await.2, "found"), "true");
    let (status, hs, _) = call(&router, from_page("/auth/wait", &format!(r#"{{"handle":"{handle}"}}"#))).await;
    assert_eq!(status, 403);
    assert!(hs.get(header::SET_COOKIE).is_none());
    // Nobody else's code is answerable.
    let other = from_command("/auth/confirm", r#"{"code":"someone else's","accept":true}"#);
    assert_eq!(field(&call(&router, other).await.2, "found"), "false");
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
        "script-src 'self';",
        "object-src 'none'",
        "base-uri 'none'",
        "form-action 'self'",
        "frame-ancestors 'none'",
        "connect-src 'self' ws://127.0.0.1:7777 ws://127.0.0.1:8080;",
    ] {
        assert!(csp.contains(directive), "{directive} in {csp}");
    }
    assert!(!csp.contains("script-src 'self' 'unsafe-inline'") && !csp.contains("'unsafe-eval'"), "{csp}");
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
