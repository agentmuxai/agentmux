// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The process around the request loop, run for real against a stand-in srv: how it reads its
//! environment (the block it scopes calls to, the agent token it sends), that it answers on
//! stdout and that it ends when its input does. Pinned so `run` can be split without changing
//! any of it.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One request the stand-in srv got: its path, headers (names lowercased) and JSON body.
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// A srv that answers every request with `{"shell_id":"sh1"}` and records it.
fn stub_srv() -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let log = log.clone();
            std::thread::spawn(move || {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let head_end = loop {
                    if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                    match conn.read(&mut buf) {
                        Ok(n) if n > 0 => raw.extend_from_slice(&buf[..n]),
                        _ => return,
                    }
                };
                let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
                let path = head.lines().next().unwrap_or_default().split(' ').nth(1).unwrap_or_default().to_string();
                let headers: Vec<(String, String)> = head
                    .lines()
                    .skip(1)
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                    .collect();
                let length = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse::<usize>().ok()).unwrap_or(0);
                while raw.len() < head_end + length {
                    match conn.read(&mut buf) {
                        Ok(n) if n > 0 => raw.extend_from_slice(&buf[..n]),
                        _ => break,
                    }
                }
                let body = serde_json::from_slice(&raw[head_end..]).unwrap_or(Value::Null);
                log.lock().unwrap().push(Seen { path, headers, body });
                let answer = r#"{"shell_id":"sh1"}"#;
                let _ = write!(
                    conn,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                );
            });
        }
    });
    (url, seen)
}

/// Runs `agentmux-mcp` with `env` (and the stand-in srv's address and key) as its only AgentMux
/// environment, feeds it `requests`, closes its input and returns its answers by request id,
/// and what srv saw.
fn run_mcp(env: &[(&str, &str)], requests: &[Value]) -> (Vec<Value>, Arc<Mutex<Vec<Seen>>>) {
    let (url, seen) = stub_srv();
    let mut command = Command::new(env!("CARGO_BIN_EXE_agentmux-mcp"));
    // Nothing of the AgentMux this test runs in may reach the process.
    for (key, _) in std::env::vars().filter(|(k, _)| k.starts_with("AGENTMUX_")) {
        command.env_remove(key);
    }
    let mut child = command
        .env("AGENTMUX_LOCAL_URL", &url)
        .env("AGENTMUX_AUTH_KEY", "key")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("agentmux-mcp starts");
    let mut stdin = child.stdin.take().unwrap();
    for r in requests {
        writeln!(stdin, "{r}").unwrap();
    }
    drop(stdin);
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        let _ = tx.send(out);
    });
    let out = match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(out) => out,
        Err(_) => {
            let _ = child.kill();
            panic!("agentmux-mcp did not end when its input closed");
        }
    };
    let status = child.wait().unwrap();
    assert!(status.success(), "{status}");
    let mut answers: Vec<Value> = out.lines().map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not json ({e}): {l:?}"))).collect();
    answers.sort_by_key(|a| a["id"].as_u64().unwrap_or(0));
    (answers, seen)
}

/// A call that starts a background job, which srv scopes to the agent's block.
fn shell(id: u64) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": "Terminal", "arguments": { "action": "run", "command": "ls", "background": true } } })
}

fn shell_create(seen: &Arc<Mutex<Vec<Seen>>>) -> Seen {
    let mut seen = seen.lock().unwrap();
    let at = seen.iter().position(|s| s.path == "/api/v1/shell/create").expect("the call reached srv");
    seen.remove(at)
}

#[test]
fn it_answers_initialize_and_the_tool_list_and_ends_when_its_input_does() {
    let (answers, _) = run_mcp(
        &[],
        &[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "nope" }),
        ],
    );
    assert_eq!(answers.len(), 3);
    assert_eq!(answers[0]["result"]["serverInfo"]["name"], "agentmux-mcp");
    let tools: Vec<&str> = answers[1]["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(tools.contains(&"Terminal") && tools.contains(&"Message"), "{tools:?}");
    assert_eq!(answers[2]["error"]["code"], -32601);
}

#[test]
fn a_call_goes_to_the_srv_in_the_environment_scoped_to_the_agents_block() {
    let (answers, seen) = run_mcp(&[("AGENTMUX_BLOCKID", "blk-1"), ("AGENTMUX_AGENT_BUS_ID", "claude")], &[shell(1)]);
    assert_eq!(answers[0]["result"]["isError"], false, "{}", answers[0]);
    assert!(answers[0]["result"]["content"][0]["text"].as_str().unwrap().contains("sh1"), "{}", answers[0]);
    let create = shell_create(&seen);
    assert_eq!(create.body["agent_block_id"], "blk-1");
    assert_eq!(create.header("x-authkey"), Some("key"));
}

#[test]
fn the_bus_id_scopes_a_call_only_when_there_is_no_block_id() {
    for env in [vec![("AGENTMUX_AGENT_BUS_ID", "bus-1")], vec![("AGENTMUX_BLOCKID", ""), ("AGENTMUX_AGENT_BUS_ID", "bus-1")]] {
        let (_, seen) = run_mcp(&env, &[shell(1)]);
        assert_eq!(shell_create(&seen).body["agent_block_id"], "bus-1", "{env:?}");
    }
}

#[test]
fn the_agent_token_is_sent_on_every_request_trimmed_and_only_when_there_is_one() {
    let (_, seen) = run_mcp(&[("AGENTMUX_BLOCKID", "b"), ("AGENTMUX_AGENT_TOKEN", "  tok-1  ")], &[shell(1)]);
    let seen = seen.lock().unwrap();
    assert!(seen.len() >= 2, "more than the call reached srv");
    for s in seen.iter() {
        assert_eq!(s.header("x-agent-token"), Some("tok-1"), "{}", s.path);
    }
    drop(seen);
    for token in [None, Some("   ")] {
        let env: Vec<(&str, &str)> = [("AGENTMUX_BLOCKID", "b")].into_iter().chain(token.map(|t| ("AGENTMUX_AGENT_TOKEN", t))).collect();
        let (_, seen) = run_mcp(&env, &[shell(1)]);
        for s in seen.lock().unwrap().iter() {
            assert_eq!(s.header("x-agent-token"), None, "{token:?} {}", s.path);
        }
    }
}

#[test]
fn a_tool_it_does_not_have_is_answered_as_an_error() {
    let (answers, seen) = run_mcp(
        &[("AGENTMUX_BLOCKID", "b")],
        &[json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "NoSuchTool", "arguments": {} } })],
    );
    assert_eq!(answers[0]["result"]["isError"], true, "{}", answers[0]);
    assert!(seen.lock().unwrap().iter().all(|s| s.path != "/api/v1/shell/create"));
}
