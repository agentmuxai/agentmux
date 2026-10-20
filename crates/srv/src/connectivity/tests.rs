// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use super::*;

#[test]
fn a_cli_retry_with_no_status_is_a_failed_connection() {
    let frame = json!({ "type": "system", "subtype": "api_retry", "attempt": 2, "max_retries": 10, "error_status": null, "error": "unknown" });
    assert_eq!(claude_frame_outcome(&frame), Some(Outcome::Unreachable("Claude CLI retrying: unknown".into())));
}

#[test]
fn a_cli_retry_with_a_status_was_answered() {
    let frame = json!({ "type": "system", "subtype": "api_retry", "attempt": 1, "error_status": 529, "error": "overloaded_error" });
    assert_eq!(claude_frame_outcome(&frame), Some(Outcome::Answered));
}

#[test]
fn a_models_reply_was_answered_and_the_clis_own_error_text_is_not() {
    let reply = json!({ "type": "assistant", "message": { "model": "claude-opus-4-1", "content": [] } });
    assert_eq!(claude_frame_outcome(&reply), Some(Outcome::Answered));
    let synthetic = json!({ "type": "assistant", "message": { "model": "<synthetic>", "content": [{ "type": "text", "text": "API Error: Connection refused" }] } });
    assert_eq!(claude_frame_outcome(&synthetic), None);
    for other in [
        json!({ "type": "system", "subtype": "init" }),
        json!({ "type": "user", "message": {} }),
        json!({ "type": "result", "is_error": true, "result": "API Error: 401 invalid x-api-key" }),
        json!({ "type": "result", "is_error": false, "result": "done" }),
    ] {
        assert_eq!(claude_frame_outcome(&other), None, "{other}");
    }
}

#[test]
fn a_turn_that_ended_on_a_refused_connection_is_a_failed_connection() {
    let frame = json!({
        "type": "result", "subtype": "success", "is_error": true,
        "result": "API Error: Connection refused — a firewall or proxy may be blocking it (ECONNREFUSED)",
    });
    assert!(matches!(claude_frame_outcome(&frame), Some(Outcome::Unreachable(_))));
}

#[test]
fn a_websocket_handshake_answer_is_reachable_and_an_io_error_is_not() {
    use tokio_tungstenite::tungstenite::Error;
    assert_eq!(ws_connect_outcome(None), Some(Outcome::Answered));
    let refused = Error::Io(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused"));
    assert!(matches!(ws_connect_outcome(Some(&refused)), Some(Outcome::Unreachable(_))));
    let rejected = Error::Http(tokio_tungstenite::tungstenite::http::Response::builder().status(401).body(None).unwrap());
    assert_eq!(ws_connect_outcome(Some(&rejected)), Some(Outcome::Answered), "a 401 came from the server");
}

#[tokio::test]
async fn an_http_answer_of_any_status_is_reachable_and_a_refused_connect_is_not() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf).await;
        let _ = sock.write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(http_outcome(&http.get(format!("http://{addr}/")).send().await), Some(Outcome::Answered));

    // A port nothing listens on: bound, then released.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let refused = http.get(format!("http://{closed}/")).send().await;
    assert!(matches!(http_outcome(&refused), Some(Outcome::Unreachable(_))));
}

#[test]
fn the_service_ids_and_labels_are_what_the_status_bar_reads() {
    assert_eq!(Service::ModelApi(&ANTHROPIC).id(), "model_api:anthropic");
    assert_eq!(Service::ModelApi(&ANTHROPIC).label(), "Anthropic API");
    assert_eq!(Service::Muxbus.id(), "muxbus");
}

#[test]
fn the_event_goes_out_under_its_name_with_the_status() {
    use crate::backend::mps_events::{is_srv_event, ConnectivityChanged};
    let shared = Shared::new(0);
    let changed = shared.tracker().report(Service::Muxbus, Outcome::Unreachable("dns".into()), 5).unwrap();
    let event = EventBus::typed_ws_event::<ConnectivityChanged>(&changed, String::new());
    assert_eq!(event.eventtype, EVENT_CONNECTIVITY_CHANGED);
    assert_eq!(event.data.unwrap()["services"][1]["state"], json!("degraded"));
    assert!(is_srv_event(EVENT_CONNECTIVITY_CHANGED), "only srv publishes it");
}
