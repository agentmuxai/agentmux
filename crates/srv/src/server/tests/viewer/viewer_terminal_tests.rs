// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The terminal feed: off until shared, terminals only, a hidden agent's
//! shell hidden with it, the tail from a line start, every write once, the
//! PTY's size, resume, truncation, and closing when sharing goes off.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use base64::Engine as _;
use futures_util::StreamExt;
use tower::ServiceExt;

use super::*;
use crate::backend::viewer::pairing;
use crate::server::tests::test_state;

fn share(state: &AppState, on: bool) {
    let mut settings = state.config_watcher.get_settings();
    settings.extra.insert(SETTING_SHARE_TERMINALS.to_string(), serde_json::json!(on));
    state.config_watcher.update_settings(settings);
}

fn pair_device(state: &AppState) -> String {
    let token = pairing::new_token();
    let device_id = uuid::Uuid::new_v4().to_string();
    state.mstore.viewer_device_insert(&device_id, &pairing::token_hash(&token), "Pixel", "", 1).unwrap();
    token
}

/// A block of `view`, with extra meta.
fn block(state: &AppState, view: &str, extra: &[(&str, &str)]) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let mut b = crate::backend::obj::Block { oid: id.clone(), ..Default::default() };
    b.meta.insert("view".into(), serde_json::json!(view));
    for (k, v) in extra {
        b.meta.insert((*k).into(), serde_json::json!(v));
    }
    state.mstore.insert(&mut b).unwrap();
    id
}

/// A PTY write, the way the shell controller's flusher makes one.
fn write(state: &AppState, block_id: &str, data: &[u8]) {
    crate::backend::blockcontroller::shell::handle_append_block_file(
        &state.broker,
        block_id,
        TERM_FILE,
        data,
        Some(&state.filestore),
        None,
    );
}

async fn status_of(state: &AppState, uri: &str, token: Option<&str>) -> StatusCode {
    let mut b = Request::builder().method(Method::GET).uri(uri);
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    let resp = build_viewer_router(state.clone()).oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    resp.status()
}

fn feed_uri(block_id: &str) -> String {
    format!("/agentmux/viewer/terminals/{block_id}/feed")
}

/// Fast clocks, so heartbeats and size polls happen within a test.
fn fast() -> TermTiming {
    TermTiming { heartbeat: Duration::from_millis(40), size_poll: Duration::from_millis(20) }
}

fn open(state: &AppState, block_id: &str, last_event_id: Option<u64>) -> impl Stream<Item = Result<Bytes, Infallible>> {
    let lease = state.viewer.open_feed("dev").unwrap();
    terminal_stream(TermSource::of(state, block_id), last_event_id, lease, fast(), TermProbe::of(state, block_id))
}

/// The next event's `(name, data)`, heartbeats and the preamble skipped.
async fn next_event<S>(stream: &mut S) -> Option<(String, serde_json::Value)>
where
    S: Stream<Item = Result<Bytes, Infallible>> + Unpin,
{
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next()).await.ok()??;
        let text = String::from_utf8(chunk.unwrap().to_vec()).unwrap();
        if text.starts_with(':') || text.starts_with("retry:") {
            continue;
        }
        let name = text.lines().find_map(|l| l.strip_prefix("event: "))?.to_string();
        let data = text.lines().find_map(|l| l.strip_prefix("data: ")).map(|d| serde_json::from_str(d).unwrap())?;
        return Some((name, data));
    }
}

fn bytes_of(data: &serde_json::Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(data["data64"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn terminals_are_off_until_shared_and_only_terminals_open() {
    let state = test_state();
    let token = pair_device(&state);
    let term = block(&state, "term", &[]);
    let agent = block(&state, "agent", &[]);

    assert_eq!(status_of(&state, &feed_uri(&term), None).await, StatusCode::UNAUTHORIZED, "the viewer token is needed");
    assert_eq!(status_of(&state, &feed_uri(&term), Some(&token)).await, StatusCode::FORBIDDEN, "off until shared");
    share(&state, true);
    assert_eq!(status_of(&state, &feed_uri("no-such-block"), Some(&token)).await, StatusCode::NOT_FOUND);
    assert_eq!(status_of(&state, &feed_uri(&agent), Some(&token)).await, StatusCode::NOT_FOUND, "not a terminal");
    assert_eq!(state.viewer.open_feeds(), 0, "a refused feed holds no place");
}

#[tokio::test]
async fn a_hidden_agents_shell_is_hidden_with_it() {
    let state = test_state();
    share(&state, true);
    let token = pair_device(&state);
    let drawer = block(&state, "term", &[]);
    let def = uuid::Uuid::new_v4().to_string();
    let mut d = crate::backend::storage::agents::test_agent_def(&def, "Secret", "claude", "standalone", 1, "");
    state.mstore.agent_def_insert(&mut d).unwrap();
    let key = crate::backend::blockcontroller::META_KEY_SHELL_SUBBLOCK_ID;
    let _agent = block(&state, "agent", &[("agentId", &def), (key, &drawer)]);
    assert_eq!(terminal_watchable(&state, &drawer), Ok(()), "shown while its agent is");
    state.mstore.agent_hide_from_devices_set(&def, true).unwrap();
    assert_eq!(terminal_watchable(&state, &drawer), Err(Unwatchable::Hidden));
    assert_eq!(status_of(&state, &feed_uri(&drawer), Some(&token)).await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_terminal_feed_sends_its_output_then_each_write_once_with_the_ptys_size() {
    let state = test_state();
    share(&state, true);
    let term = block(&state, "term", &[]);
    write(&state, &term, b"$ ls\r\n");
    write(&state, &term, b"a  b\r\n");
    crate::backend::blockcontroller::term_size::record(&term, 30, 100);
    let mut stream = Box::pin(open(&state, &term, None));

    let (event, snap) = next_event(&mut stream).await.unwrap();
    assert_eq!(event, "snapshot");
    assert_eq!((snap["offset"].as_u64(), snap["next_offset"].as_u64()), (Some(0), Some(12)));
    assert_eq!(bytes_of(&snap), b"$ ls\r\na  b\r\n", "from the file's start: all of it");
    assert_eq!((snap["rows"].as_u64(), snap["cols"].as_u64()), (Some(30), Some(100)));

    write(&state, &term, b"\x1b[32mok\x1b[0m");
    let (event, data) = next_event(&mut stream).await.unwrap();
    assert_eq!((event.as_str(), data["offset"].as_u64()), ("append", Some(12)));
    assert_eq!(bytes_of(&data), b"\x1b[32mok\x1b[0m", "raw bytes, escapes and all");

    // The desktop resizes the PTY.
    crate::backend::blockcontroller::term_size::record(&term, 40, 120);
    let (event, size) = next_event(&mut stream).await.unwrap();
    assert_eq!((event.as_str(), size["rows"].as_u64(), size["cols"].as_u64()), ("size", Some(40), Some(120)));
    crate::backend::blockcontroller::term_size::forget(&term);
}

#[tokio::test]
async fn a_long_terminals_snapshot_is_its_tail_from_a_line_start() {
    let state = test_state();
    share(&state, true);
    let term = block(&state, "term", &[]);
    let line = format!("{}\r\n", "x".repeat(98));
    for _ in 0..4000 {
        write(&state, &term, line.as_bytes());
    }
    let size = 4000 * line.len() as u64;
    let mut stream = Box::pin(open(&state, &term, None));
    let (_, snap) = next_event(&mut stream).await.unwrap();
    let offset = snap["offset"].as_u64().unwrap();
    assert_eq!(snap["next_offset"].as_u64(), Some(size));
    assert!(size - offset <= crate::backend::viewer::terminal::SNAPSHOT_TAIL_BYTES, "a bounded tail");
    assert_eq!(offset % line.len() as u64, 0, "starts at a line, not mid-line");
    assert_eq!(bytes_of(&snap).len() as u64, size - offset);
}

#[tokio::test]
async fn a_reconnect_gets_what_it_missed_and_a_stale_one_a_snapshot() {
    let state = test_state();
    share(&state, true);
    let term = block(&state, "term", &[]);
    write(&state, &term, b"abcdef");
    let mut stream = Box::pin(open(&state, &term, Some(2)));
    let (event, data) = next_event(&mut stream).await.unwrap();
    assert_eq!((event.as_str(), data["offset"].as_u64()), ("append", Some(2)), "no snapshot for a resume that holds");
    assert_eq!(bytes_of(&data), b"cdef");
    drop(stream);

    let mut stream = Box::pin(open(&state, &term, Some(99)));
    let (event, snap) = next_event(&mut stream).await.unwrap();
    assert_eq!(event, "snapshot", "past the end: the terminal was cleared");
    assert_eq!(bytes_of(&snap), b"abcdef");
}

#[tokio::test]
async fn a_truncated_terminal_resets_and_sends_a_fresh_snapshot() {
    let state = test_state();
    share(&state, true);
    let term = block(&state, "term", &[]);
    write(&state, &term, b"old output\n");
    let mut stream = Box::pin(open(&state, &term, None));
    assert_eq!(next_event(&mut stream).await.unwrap().0, "snapshot");

    state.filestore.write_file(&term, TERM_FILE, b"").unwrap();
    let event = crate::backend::mps::WSFileEventData {
        zoneid: term.clone(),
        filename: TERM_FILE.into(),
        fileop: crate::backend::mps::FILE_OP_TRUNCATE.into(),
        data64: String::new(),
        offset: None,
        pos: Vec::new(),
        echo: None,
    };
    state.broker.publish(crate::backend::mps::MuxEvent {
        event: crate::backend::mps::EVENT_BLOCK_FILE.into(),
        scopes: vec![format!("block:{term}")],
        sender: String::new(),
        persist: 0,
        data: serde_json::to_value(&event).ok(),
    });
    let (event, reset) = next_event(&mut stream).await.unwrap();
    assert_eq!((event.as_str(), reset["reason"].as_str()), ("reset", Some(crate::backend::mps::FILE_OP_TRUNCATE)));
    let (event, snap) = next_event(&mut stream).await.unwrap();
    assert_eq!(event, "snapshot");
    assert!(bytes_of(&snap).is_empty());
    write(&state, &term, b"new\n");
    let (event, data) = next_event(&mut stream).await.unwrap();
    assert_eq!((event.as_str(), data["offset"].as_u64()), ("append", Some(0)));
}

#[tokio::test]
async fn turning_sharing_off_closes_an_open_terminal_feed() {
    let state = test_state();
    share(&state, true);
    let term = block(&state, "term", &[]);
    let mut stream = Box::pin(open(&state, &term, None));
    assert_eq!(next_event(&mut stream).await.unwrap().0, "snapshot");
    share(&state, false);
    let (event, reset) = next_event(&mut stream).await.unwrap();
    assert_eq!((event.as_str(), reset["reason"].as_str()), ("reset", Some("not_shared")));
    assert!(tokio::time::timeout(Duration::from_secs(5), stream.next()).await.unwrap().is_none(), "closed");
    assert_eq!(state.viewer.open_feeds(), 0);
}
