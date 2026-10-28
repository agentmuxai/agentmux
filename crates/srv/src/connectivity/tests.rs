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

fn failed(exit: Option<i32>, signal: Option<i32>, text: &str) -> crate::agents::failure::AgentFailure {
    crate::agents::failure::classify(exit, signal, text, None)
}

#[test]
fn codex_and_gemini_turns_speak_for_their_own_api() {
    let (api, outcome) = provider_turn_outcome("codex", None).unwrap();
    assert_eq!((api.vendor, outcome), ("openai", Outcome::Answered));
    let down = failed(Some(1), None, "APIConnectionError: Connection error.");
    assert_eq!(down.code, crate::agents::failure::FailureClass::Network);
    let (api, outcome) = provider_turn_outcome("gemini", Some(&down)).unwrap();
    assert_eq!(api.vendor, "google");
    assert!(matches!(outcome, Outcome::Unreachable(_)));
    // The API itself returned these, so it was reached.
    for text in ["Error: HTTP 429 Too Many Requests", "You've hit your weekly limit", "request failed with status 401", "overloaded_error: please retry"] {
        let f = failed(Some(1), None, text);
        assert_eq!(provider_turn_outcome("codex", Some(&f)).map(|(_, o)| o), Some(Outcome::Answered), "{text}");
    }
    // Others say nothing about the network.
    for f in [failed(None, Some(9), ""), failed(Some(2), None, "something unexpected happened")] {
        assert!(provider_turn_outcome("codex", Some(&f)).is_none(), "{:?}", f.code);
    }
}

#[test]
fn claude_and_untracked_providers_are_left_to_their_own_reporting() {
    assert!(provider_turn_outcome("claude", None).is_none(), "Claude reports from its frames");
    assert!(provider_turn_outcome("aider", None).is_none());
    assert!(provider_turn_outcome("", None).is_none());
}

fn env(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn a_pane_with_no_address_of_its_own_or_srvs_address_uses_the_providers_service() {
    let own = Service::ModelApi(&ANTHROPIC).probe_url();
    assert_eq!(service_for(&ANTHROPIC, None), Service::ModelApi(&ANTHROPIC));
    assert_eq!(service_for(&ANTHROPIC, Some("  ")), Service::ModelApi(&ANTHROPIC));
    assert_eq!(service_for(&ANTHROPIC, Some(&format!("{own}/"))), Service::ModelApi(&ANTHROPIC), "the same address, written another way");
    assert_eq!(service_for_env(&ANTHROPIC, &env(&[("PATH", "x")])), Service::ModelApi(&ANTHROPIC));
}

#[test]
fn a_pane_pointed_at_a_proxy_is_its_own_service_probed_at_the_proxy() {
    let service = service_for_env(&ANTHROPIC, &env(&[("ANTHROPIC_BASE_URL", "https://user:secret@proxy.example.com:8443/v1/?key=abc")]));
    assert_eq!(service, Service::ModelApiAt(&ANTHROPIC, "https://proxy.example.com:8443/v1".to_string()));
    assert_eq!(service.id(), "model_api:anthropic@proxy.example.com:8443");
    assert_eq!(service.label(), "Anthropic API (proxy.example.com:8443)");
    assert_eq!(service.probe_url(), "https://proxy.example.com:8443/v1");
    let shown = format!("{} {}", service.id(), service.label());
    assert!(!shown.contains("secret") && !shown.contains("abc"), "no credential in what the status shows: {shown}");
}

#[test]
fn only_the_variable_the_provider_reads_sets_its_address() {
    let service = service_for_env(&OPENAI, &env(&[("ANTHROPIC_BASE_URL", "https://proxy.example.com")]));
    assert_eq!(service, Service::ModelApi(&OPENAI));
    let service = service_for_env(&GOOGLE, &env(&[("GOOGLE_GEMINI_BASE_URL", "https://gw.example.com")]));
    assert_eq!(service.id(), "model_api:google@gw.example.com");
}

#[test]
fn the_pane_is_told_its_service_only_when_it_is_not_the_default() {
    let at = service_for(&ANTHROPIC, Some("https://proxy.example.com"));
    assert_eq!(endpoint_meta(&at), Some(json!({ "service": "model_api:anthropic@proxy.example.com", "url": "https://proxy.example.com" })));
    assert_eq!(endpoint_meta(&Service::ModelApi(&ANTHROPIC)), None);
    assert_eq!(endpoint_meta(&Service::Muxbus), None);
}

#[test]
fn a_proxy_and_the_default_api_have_their_own_rows_and_states() {
    let shared = Shared::new(0);
    let proxy = service_for(&ANTHROPIC, Some("https://proxy.example.com"));
    shared.report_at(Service::ModelApi(&ANTHROPIC), Outcome::Answered, 1);
    for at in 2..5 {
        shared.report_at(proxy.clone(), Outcome::Unreachable("refused".into()), at);
    }
    let rows: Vec<(String, Reachability)> = shared.tracker().status().services.into_iter().map(|s| (s.service, s.state)).collect();
    assert!(rows.contains(&("model_api:anthropic".to_string(), Reachability::Online)), "{rows:?}");
    assert!(rows.contains(&("model_api:anthropic@proxy.example.com".to_string(), Reachability::Offline)), "{rows:?}");
}

#[test]
fn tracking_a_panes_address_lists_it_as_not_checked_yet_without_touching_one_that_reported() {
    let shared = Shared::new(0);
    let proxy = service_for(&ANTHROPIC, Some("https://proxy.example.com"));
    shared.track_at(proxy.clone(), 1);
    let state = |id: &str| shared.tracker().status().services.into_iter().find(|s| s.service == id).map(|s| s.state);
    assert_eq!(state("model_api:anthropic@proxy.example.com"), Some(Reachability::Unknown));
    shared.report_at(proxy.clone(), Outcome::Answered, 2);
    shared.track_at(proxy, 3);
    assert_eq!(state("model_api:anthropic@proxy.example.com"), Some(Reachability::Online), "a second spawn does not forget what is known");
}

#[test]
fn a_providers_model_api_is_found_from_its_first_vendor() {
    assert_eq!(model_api_of("claude"), Some(&ANTHROPIC));
    assert_eq!(model_api_of("codex"), Some(&OPENAI));
    assert_eq!(model_api_of("gemini"), Some(&GOOGLE));
    assert_eq!(model_api_of("no-such-provider"), None);
}

#[test]
fn an_address_that_cannot_be_read_is_no_address_and_none_of_it_is_kept() {
    for raw in ["user:secret@proxy.example.com:8080", "gw.example.com/v1?key=abc", "not a url"] {
        let service = service_for(&ANTHROPIC, Some(raw));
        assert_eq!(service, Service::ModelApi(&ANTHROPIC), "{raw}");
        assert_eq!(endpoint_meta(&service), None);
    }
}
