// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn called(name: &str, session: Option<&str>, context: i64, payload: &str) -> Vec<u8> {
    let mut m = serde_json::json!({
        "method": "Runtime.bindingCalled",
        "params": { "name": name, "payload": payload, "executionContextId": context },
    });
    if let Some(s) = session {
        m["sessionId"] = serde_json::json!(s);
    }
    m.to_string().into_bytes()
}

const SIGNIN: &str = r#"{"kind":"signin","username":"alex","password":"correct horse"}"#;

#[test]
fn only_an_http_origin_with_a_host_is_savable() {
    for ok in ["https://accounts.example.com", "http://localhost:8080"] {
        assert!(savable(ok), "{ok}");
    }
    for bad in ["", "null", "://", "about:blank", "data:", "file://", "https://", "https://a.example/path", "chrome-extension://abc"] {
        assert!(!savable(bad), "{bad}");
    }
}

#[test]
fn our_binding_s_call_is_taken_and_nothing_else_is() {
    // Ours: taken, even with no world noted for it (then dropped).
    assert!(take(910_001, &called(BINDING, None, 3, SIGNIN)));
    // Someone else's binding, another event, a console line naming ours, garbage: handed on.
    assert!(!take(910_001, &called("agentmuxSideButton", None, 3, "{}")));
    assert!(!take(910_001, br#"{"method":"Page.loadEventFired","params":{}}"#));
    let console = serde_json::json!({ "method": "Runtime.consoleAPICalled", "params": { "args": [{ "value": BINDING }] } });
    assert!(!take(910_001, console.to_string().as_bytes()));
    assert!(!take(910_001, b"agentmuxSignIn but not json"));
}

#[test]
fn a_sign_in_is_named_by_the_world_it_came_from_never_by_the_page() {
    note_world(910_002, None, "main", 7, "https://accounts.example.com");
    note_world(910_002, Some("S1"), "child", 7, "https://widget.example.net");
    note_world(910_002, None, "blank", 9, "://");
    assert_eq!(origin_of(910_002, None, 7).as_deref(), Some("https://accounts.example.com"));
    assert_eq!(origin_of(910_002, Some("S1"), 7).as_deref(), Some("https://widget.example.net"), "the same context id in another session is another frame");
    assert_eq!(origin_of(910_002, None, 9), None, "an opaque frame is refused");
    assert_eq!(origin_of(910_002, None, 8), None);
    // A frame's new document replaces its world.
    note_world(910_002, None, "main", 11, "https://accounts.example.com");
    assert_eq!(origin_of(910_002, None, 7), None);
    // Frames that left the page are forgotten.
    keep_frames(910_002, &["child".to_string()].into());
    assert_eq!(origin_of(910_002, None, 11), None);
    assert!(origin_of(910_002, Some("S1"), 7).is_some());
    forget(910_002);
    assert_eq!(origin_of(910_002, Some("S1"), 7), None);
}

#[test]
fn the_listener_reports_four_kinds() {
    assert!(matches!(serde_json::from_str::<Notice>(SIGNIN), Ok(Notice::Signin { .. })));
    assert!(matches!(serde_json::from_str::<Notice>(r#"{"kind":"identifier","username":"alex"}"#), Ok(Notice::Identifier { .. })));
    assert!(matches!(serde_json::from_str::<Notice>(r#"{"kind":"left"}"#), Ok(Notice::Left)));
    assert!(matches!(serde_json::from_str::<Notice>(r#"{"kind":"form"}"#), Ok(Notice::Form)));
    assert!(serde_json::from_str::<Notice>(r#"{"kind":"signin","username":"alex"}"#).is_err(), "a sign-in without a password");
    assert!(serde_json::from_str::<Notice>(r#"{"kind":"other"}"#).is_err());
}

#[test]
fn the_listener_counts_only_the_persons_own_sign_ins() {
    assert!(SCRIPT.contains(&format!("window.{BINDING}(")));
    for needle in ["e.isTrusted", "new-password", r#"type === "password""#, "capture: true", "\"submit\"", "\"keydown\"", "\"click\"", "personAt < PERSON_MS"] {
        assert!(SCRIPT.contains(needle), "{needle}");
    }
}

#[tokio::test]
async fn a_panes_notices_reach_srv_in_the_order_they_were_seen() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let tx = in_order(&tokio::runtime::Handle::current(), move |method, _| {
        let log = log.clone();
        async move {
            // The offer's request is the slower one: the settle must still wait for it.
            if method == "Offer" {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            log.lock().push(method);
        }
    });
    tx.send(("Offer", serde_json::json!({}))).unwrap();
    tx.send(("Settled", serde_json::json!({}))).unwrap();
    drop(tx);
    for _ in 0..100 {
        if seen.lock().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(*seen.lock(), ["Offer", "Settled"]);
}

#[test]
fn a_sign_in_is_settled_by_the_frame_it_was_sent_from() {
    let b = 910_031;
    note_world(b, Some("S1"), "IFRAME-1", 12, "https://login.example.net");
    // No pane: nothing is sent, but the frame is still noted.
    assert!(take(b, &called(BINDING, Some("S1"), 12, SIGNIN)));
    assert!(signed_in_from(b, "IFRAME-1"));
    assert!(!signed_in_from(b, "MAIN"), "another frame's navigation says nothing about it");
    forget(b);
    assert!(!signed_in_from(b, "IFRAME-1"));
}
