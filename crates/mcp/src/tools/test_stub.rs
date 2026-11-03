// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A stand-in srv for the tests that pin what a tool sends and says: it records each request
//! (path with its query, and the JSON body) and answers every one with a fixed status and body.

use super::*;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

pub(super) struct StubSrv {
    pub(super) url: String,
    pub(super) seen: Arc<Mutex<Vec<(String, Value)>>>,
}

fn read_request(conn: &mut std::net::TcpStream) -> (String, Value) {
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = conn.read(&mut buf).unwrap();
        assert!(n > 0, "connection closed before the headers ended");
        raw.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
    let path = head.lines().next().unwrap_or_default().split(' ').nth(1).unwrap_or_default().to_string();
    let length = head
        .to_ascii_lowercase()
        .lines()
        .find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
        .unwrap_or(0);
    while raw.len() < head_end + length {
        let n = conn.read(&mut buf).unwrap();
        assert!(n > 0, "connection closed before the body ended");
        raw.extend_from_slice(&buf[..n]);
    }
    (path, serde_json::from_slice(&raw[head_end..]).unwrap_or(Value::Null))
}

/// A srv that answers every request with `status` (e.g. `"200 OK"`) and `reply`.
pub(super) fn stub_srv(status: &'static str, reply: Value) -> StubSrv {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let mut conn = conn.unwrap();
            let log = log.clone();
            let answer = reply.to_string();
            std::thread::spawn(move || {
                let request = read_request(&mut conn);
                log.lock().unwrap().push(request);
                let _ = write!(
                    conn,
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                );
            });
        }
    });
    StubSrv { url, seen }
}

/// A tool context pointed at the stub, as an agent in a container pane (block `blk`, agent
/// `tester`) has it. `AGENTMUX_AGENT_ID` and the container hint are process-wide, so it holds
/// the same lock as the key tests until dropped.
pub(super) struct Fixture {
    _guard: std::sync::MutexGuard<'static, ()>,
    url: String,
    client: reqwest::Client,
    loops: LoopRegistry,
    counter: std::sync::atomic::AtomicU64,
}

impl Fixture {
    pub(super) fn new(srv: &StubSrv) -> Self {
        let guard = crate::self_keys::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("AGENTMUX_AGENT_ID", "tester");
        std::env::set_var("AGENTMUX_IN_CONTAINER", "1");
        std::env::remove_var("AGENTMUX_JEKT_KEY");
        std::env::remove_var("AGENTMUX_AGENT_UID");
        Self { _guard: guard, url: srv.url.clone(), client: reqwest::Client::new(), loops: LoopRegistry::default(), counter: Default::default() }
    }

    pub(super) fn cx(&self) -> ToolCtx<'_> {
        ToolCtx { local_url: &self.url, auth_key: "key", block_id: "blk", client: &self.client, loops: &self.loops, loop_counter: &self.counter }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::env::remove_var("AGENTMUX_IN_CONTAINER");
    }
}
