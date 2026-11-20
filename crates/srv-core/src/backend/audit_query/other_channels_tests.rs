// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::rpc_types::AuditEntry;
use serde_json::json;

fn row(id: &str, ts_ms: u64) -> serde_json::Value {
    json!({ "id": id, "ts_ms": ts_ms, "kind": "jekt", "summary": "a message", "links": { "pane": { "block_id": "b-1" } },
            "from": { "agent_id": "alpha", "links": { "pane": { "block_id": "b-2" } } } })
}

/// One-shot HTTP server answering any request with `body`, and the request it received.
async fn serve_once(body: String) -> (String, tokio::sync::oneshot::Receiver<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else { return };
        // The headers and the body may arrive in separate reads: read until the body is all there.
        let mut got = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            got.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&got).to_lowercase();
            let Some((head, body)) = text.split_once("\r\n\r\n") else {
                if n == 0 {
                    break;
                }
                continue;
            };
            let wanted = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
            if n == 0 || body.len() >= wanted {
                break;
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&got).into_owned());
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let _ = stream.write_all(response.as_bytes()).await;
    });
    (url, rx)
}

fn result_of(entries: Vec<serde_json::Value>, next_cursor: Option<&str>) -> String {
    json!({ "entries": entries, "next_cursor": next_cursor, "not_recorded": [], "channel": "dev" }).to_string()
}

#[test]
fn the_other_channels_get_the_filter_and_a_first_page_of_their_own_rows() {
    let req = AuditQueryReq {
        agent_id: Some("alpha".into()),
        kinds: Some(vec!["jekt".into()]),
        text: Some("x".into()),
        limit: Some(20),
        cursor: Some("5|1|a|-".into()),
        totals: Some(true),
        scope: Some("machine".into()),
        ..Default::default()
    };
    let sent = request_for_others(&req);
    assert_eq!((sent.agent_id.as_deref(), sent.text.as_deref(), sent.limit), (Some("alpha"), Some("x"), Some(20)));
    assert_eq!(sent.kinds, Some(vec!["jekt".to_string()]));
    // Its paging and totals are this srv's; asking for `machine` would make the other channel ask in turn.
    assert_eq!((sent.cursor, sent.totals, sent.scope.as_deref()), (None, None, Some("channel")));
}

#[test]
fn a_rows_id_names_its_channel_and_a_link_into_the_other_window_is_dropped() {
    let result: AuditQueryResult = serde_json::from_str(&result_of(vec![row("o1", 10)], Some("more"))).unwrap();
    let rows = rows_of("dev", result);
    assert_eq!(rows.channel, "dev");
    assert!(rows.more);
    let e: &AuditEntry = &rows.entries[0];
    assert_eq!(e.id, "dev:o1");
    assert_eq!(e.provenance.as_ref().and_then(|p| p.channel.as_deref()), Some("dev"));
    assert!(e.links.pane.is_none());
    assert!(e.from.as_ref().unwrap().links.pane.is_none());
}

#[test]
fn a_channel_a_row_names_keeps_the_name_it_wrote() {
    let mut r = row("o1", 10);
    r["provenance"] = json!({ "channel": "written-by" });
    let result: AuditQueryResult = serde_json::from_str(&result_of(vec![r], None)).unwrap();
    let rows = rows_of("dev", result);
    assert!(!rows.more);
    assert_eq!(rows.entries[0].provenance.as_ref().and_then(|p| p.channel.as_deref()), Some("written-by"));
}

#[tokio::test]
async fn asking_posts_to_each_channel_with_its_key_and_skips_one_that_fails() {
    let (url, request) = serve_once(result_of(vec![row("o2", 20), row("o1", 10)], None)).await;
    let targets = [
        ChannelStatusTarget { channel: "dev".into(), local_url: url, auth_key: "chan-key".into() },
        // Nothing listens on the discard port.
        ChannelStatusTarget { channel: "dead".into(), local_url: "http://127.0.0.1:9".into(), auth_key: "k".into() },
    ];
    let req = AuditQueryReq { agent_id: Some("alpha".into()), scope: Some("machine".into()), ..Default::default() };
    let rows = ask(&reqwest::Client::new(), &targets, &req).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].channel, "dev");
    assert_eq!(rows[0].entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["dev:o2", "dev:o1"]);
    let request = request.await.unwrap().to_lowercase();
    assert!(request.starts_with(&format!("post {OWN_ROWS_PATH} ")), "{request}");
    assert!(request.contains(&format!("{}: chan-key", agentmux_common::AUTH_KEY_HEADER.to_lowercase())), "{request}");
    assert!(request.contains(r#""scope":"channel""#) && request.contains(r#""agent_id":"alpha""#), "{request}");
}

#[tokio::test]
async fn a_channel_with_no_rows_of_its_own_adds_no_section() {
    let (url, _) = serve_once(result_of(Vec::new(), None)).await;
    let targets = [ChannelStatusTarget { channel: "dev".into(), local_url: url, auth_key: "k".into() }];
    assert!(ask(&reqwest::Client::new(), &targets, &AuditQueryReq::default()).await.is_empty());
}

#[tokio::test]
async fn an_answer_that_is_not_an_audit_page_is_skipped() {
    let (url, _) = serve_once("not json".to_string()).await;
    let targets = [ChannelStatusTarget { channel: "dev".into(), local_url: url, auth_key: "k".into() }];
    assert!(ask(&reqwest::Client::new(), &targets, &AuditQueryReq::default()).await.is_empty());
}
