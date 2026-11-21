// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/ui/browser/terminal` against a stand-in for the host: what is refused before a key
//! goes in, what the host is sent, and what comes back.

use std::sync::{Arc, Mutex as StdMutex};

use axum::extract::{Path, State as AxState};
use axum::routing::post;
use axum::Router;

use super::*;
use crate::backend::obj::Block;
use crate::server::browser_powers::Power;
use agentmux_common::api_types::{UiAutomationAuth, UiBrowserTerminalRequest};

/// What the stand-in host was asked, and what it says the page and its terminal are.
struct Host {
    seen: StdMutex<Vec<(String, Value)>>,
    page: StdMutex<String>,
    terminal: StdMutex<Value>,
    keys_error: StdMutex<Option<String>>,
}

async fn host_route(AxState(h): AxState<Arc<Host>>, Path(route): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    h.seen.lock().unwrap().push((route.clone(), body));
    match route.as_str() {
        "page_url" => Json(json!({ "ok": true, "data": { "url": h.page.lock().unwrap().clone() } })),
        "terminal_state" => Json(json!({ "ok": true, "data": h.terminal.lock().unwrap().clone() })),
        "terminal_keys" => match h.keys_error.lock().unwrap().clone() {
            Some(error) => Json(json!({ "ok": false, "error": error })),
            None => Json(json!({ "ok": true, "data": {} })),
        },
        _ => Json(json!({ "ok": false, "error": "unknown" })),
    }
}

struct Setup {
    state: AppState,
    host: Arc<Host>,
    auth: UiAutomationAuth,
    pane: String,
    agent: String,
    site: String,
}

/// An agent with its own browser pane on a granted site whose terminal shows a shell prompt, and a
/// grant of the terminal power on it.
async fn setup() -> Setup {
    let state = crate::server::tests::test_state();
    let site = format!("https://t{}.example.com", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let terminal = json!({ "count": 1, "origin": site, "screen_reader": true, "rows": ["$ ready", ""] });
    let host = Arc::new(Host {
        seen: StdMutex::new(Vec::new()),
        page: StdMutex::new(format!("{site}/console")),
        terminal: StdMutex::new(terminal),
        keys_error: StdMutex::new(None),
    });
    let app = Router::new().route("/agentmux/browser/:route", post(host_route)).with_state(host.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port, token: "host-token".into() });

    let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    block.meta.insert("view".into(), json!("browser"));
    state.mstore.insert(&mut block).unwrap();
    let agent = format!("typist-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&agent, &block.oid, None).unwrap();
    let key = state.mstore.agent_jekt_key_ensure(&agent).unwrap();
    let ts = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", &agent, "__srv__", ts, "");
    let auth = UiAutomationAuth { agent_id: agent.clone(), ts_secs: ts, sig };
    Setup { state, host, auth, pane: block.oid, agent, site }
}

impl Setup {
    /// The person's grant of the terminal power on the pane's site.
    fn allow_terminal(&self) {
        bypass::grant(&self.state, &self.pane, &self.agent, vec![self.site.clone()], vec![Power::Terminal], 10, How::Allowed);
    }

    fn request(&self, text: Option<&str>, keys: Option<&[&str]>) -> UiBrowserTerminalRequest {
        UiBrowserTerminalRequest {
            auth: self.auth.clone(),
            pane: None,
            text: text.map(String::from),
            enter: None,
            keys: keys.map(|k| k.iter().map(|s| s.to_string()).collect()),
            until_text: None,
            timeout_seconds: Some(1),
            reason: None,
        }
    }

    async fn call(&self, req: UiBrowserTerminalRequest) -> (StatusCode, Value) {
        let resp = handle_ui_browser_terminal(State(self.state.clone()), None, Json(req)).await.into_response();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    /// The key names of each `terminal_keys` call the host got, in order.
    fn typed(&self) -> Vec<Vec<String>> {
        self.host
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(route, _)| route == "terminal_keys")
            .map(|(_, body)| body["keys"].as_array().unwrap().iter().map(|k| k["key"].as_str().unwrap().to_string()).collect())
            .collect()
    }
}

fn error_of(body: &Value) -> String {
    body["error"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn a_call_with_nothing_to_type_or_press_is_refused() {
    let s = setup().await;
    s.allow_terminal();
    let (status, body) = s.call(s.request(None, None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(error_of(&body).contains("give `text` to type"), "{body}");
    assert!(s.typed().is_empty());
}

#[tokio::test]
async fn text_over_the_limit_a_bad_key_and_too_many_lines_are_refused_before_anything_is_typed() {
    let s = setup().await;
    s.allow_terminal();

    let (status, body) = s.call(s.request(Some(&"a".repeat(MAX_CHARS + 1)), None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(error_of(&body).contains("characters"), "{body}");

    let (status, body) = s.call(s.request(None, Some(&["NoSuchKey"]))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let many_lines = "ls\n".repeat(MAX_LINES + 1);
    let (status, body) = s.call(s.request(Some(&many_lines), None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(error_of(&body).contains("lines"), "{body}");

    assert!(s.typed().is_empty());
}

#[tokio::test]
async fn a_line_that_looks_like_a_credential_is_never_typed() {
    let s = setup().await;
    s.allow_terminal();
    let (status, body) = s.call(s.request(Some("mysql --password=hunter2 -u root"), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(error_of(&body).contains("looks like a credential"), "{body}");
    assert!(s.typed().is_empty());
}

#[tokio::test]
async fn without_a_grant_of_the_terminal_power_nothing_is_typed() {
    let s = setup().await;
    let (status, body) = s.call(s.request(Some("ls"), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(error_of(&body).contains("no web-terminal grant"), "{body}");
    assert!(s.typed().is_empty());
}

#[tokio::test]
async fn a_page_off_the_granted_sites_pauses_the_terminal() {
    let s = setup().await;
    s.allow_terminal();
    *s.host.page.lock().unwrap() = "https://elsewhere.example.org/".to_string();
    let (status, body) = s.call(s.request(Some("ls"), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(error_of(&body).contains("off the sites the person granted"), "{body}");
    assert!(s.typed().is_empty());
}

#[tokio::test]
async fn a_page_with_no_terminal_and_a_terminal_in_another_sites_frame_are_refused() {
    let s = setup().await;
    s.allow_terminal();
    *s.host.terminal.lock().unwrap() = json!({ "count": 0 });
    let (status, body) = s.call(s.request(Some("ls"), None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(error_of(&body).contains("no web terminal found"), "{body}");

    *s.host.terminal.lock().unwrap() = json!({ "count": 1, "origin": "https://frame.example.org", "rows": [] });
    let (status, body) = s.call(s.request(Some("ls"), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(error_of(&body).contains("frame from a site the person didn't grant"), "{body}");
    assert!(s.typed().is_empty());
}

#[tokio::test]
async fn nothing_is_typed_while_the_terminal_asks_for_a_password() {
    let s = setup().await;
    s.allow_terminal();
    *s.host.terminal.lock().unwrap() = json!({ "count": 1, "origin": s.site, "screen_reader": true, "rows": ["$ sudo ls", "[sudo] password for user:"] });
    let (status, body) = s.call(s.request(Some("hunter2"), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(error_of(&body).contains("something only the person should type"), "{body}");
    assert!(s.typed().is_empty());
}

/// Text goes in key by key with an Enter after it, and what the terminal shows is handed back as
/// untrusted output.
#[tokio::test]
async fn text_is_typed_a_key_at_a_time_with_an_enter_and_the_screen_comes_back() {
    let s = setup().await;
    s.allow_terminal();
    let (status, body) = s.call(s.request(Some("echo hi"), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(s.typed(), [["e", "c", "h", "o", " ", "h", "i", "Enter"].map(String::from).to_vec()]);
    assert_eq!(body["data"]["sent"], 8);
    assert_eq!(body["data"]["of"], 8);
    assert_eq!(body["data"]["output"], json!(["$ ready"]));
    assert!(body["data"]["note"].as_str().unwrap().contains("untrusted"), "{body}");
    assert!(body["data"].get("stopped").is_none(), "{body}");
}

/// Enter follows the text unless it was asked not to or the text ended in a line break; named keys
/// follow after it.
#[tokio::test]
async fn enter_follows_the_text_unless_declined_or_already_there_and_named_keys_come_after() {
    let s = setup().await;
    s.allow_terminal();
    let declined = UiBrowserTerminalRequest { enter: Some(false), ..s.request(Some("ls"), None) };
    let (status, body) = s.call(declined).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = s.call(s.request(Some("ls\n"), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = s.call(s.request(Some("ls"), Some(&["Ctrl+C"]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let typed = s.typed();
    assert_eq!(typed[0], ["l", "s"]);
    assert_eq!(typed[1], ["l", "s", "Enter"]);
    assert_eq!(typed[2], ["l", "s", "Enter"], "a line at a time: the text and its Enter first");
    assert_eq!(typed[3].len(), 1, "then the named key on its own: {:?}", typed[3]);
}

/// A script over a chunk goes to the host in chunks of `CHUNK` keys, the target checked again between.
#[tokio::test]
async fn a_long_line_goes_to_the_host_in_chunks() {
    let s = setup().await;
    s.allow_terminal();
    let line = "x".repeat(CHUNK + 8);
    let (status, body) = s.call(s.request(Some(&line), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sizes: Vec<usize> = s.typed().iter().map(Vec::len).collect();
    assert_eq!(sizes, [CHUNK, 9], "{body}");
    assert_eq!(body["data"]["sent"], CHUNK + 9);
    let looks = s.host.seen.lock().unwrap().iter().filter(|(r, _)| r == "terminal_state").count();
    assert!(looks >= 2, "the terminal was looked at before the first chunk and again before the second");
}

/// A host that can't type ends the call with its reason.
#[tokio::test]
async fn a_host_that_cannot_type_ends_the_call_with_its_reason() {
    let s = setup().await;
    s.allow_terminal();
    *s.host.keys_error.lock().unwrap() = Some("the terminal went away".to_string());
    let (status, body) = s.call(s.request(Some("ls"), None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(error_of(&body).contains("the terminal went away"), "{body}");
}

/// A pane the person paused is not typed into: the call says so and sent nothing.
#[tokio::test]
async fn a_paused_pane_is_not_typed_into() {
    let s = setup().await;
    s.allow_terminal();
    assert!(crate::server::browser_actions::set_paused(&s.state, &s.pane, true));
    let (status, body) = s.call(s.request(Some("ls"), None)).await;
    let _ = crate::server::browser_actions::set_paused(&s.state, &s.pane, false);
    if status == StatusCode::OK {
        assert_eq!(body["data"]["sent"], 0, "{body}");
        assert!(body["data"]["stopped"].as_str().unwrap().contains("paused"), "{body}");
    } else {
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }
    assert!(s.typed().is_empty());
}
