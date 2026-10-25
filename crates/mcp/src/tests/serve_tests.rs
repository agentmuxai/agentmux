// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The request loop (`serve.rs`): one stuck call must not hold up the others
//! (#2614), and a call that never returns is answered with an error well before
//! Claude Code's own 1800 s give-up.

use super::*;
use serde_json::json;
use std::time::Duration;
use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};

/// A running loop with a scripted runner: tool "Hang" never returns, "Slow"
/// takes 0.1 s and says when it started, "Echo" answers at once.
struct Rig {
    to_server: tokio::io::DuplexStream,
    from_server: BufReader<tokio::io::DuplexStream>,
    health: Arc<Health>,
}

fn rig() -> Rig {
    let (to_server, server_in) = duplex(64 * 1024);
    let (server_out, from_server) = duplex(64 * 1024);
    let health = Health::new();
    let started = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let runner: Arc<dyn ToolRunner> = Arc::new(move |params: Value| -> CallFuture {
        let name = params["name"].as_str().unwrap_or("").to_string();
        let started = Arc::clone(&started);
        Box::pin(async move {
            match name.as_str() {
                "Hang" => std::future::pending::<CallResult>().await,
                "Slow" => {
                    let n = started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok(format!("slow {n}"))
                }
                // Blocking code, as an OS call into a window that stopped answering is.
                "Block" => {
                    std::thread::sleep(Duration::from_millis(1500));
                    Ok("late".to_string())
                }
                "Boom" => panic!("a tool panicked"),
                "Fail" => anyhow::bail!("it failed"),
                _ => Ok(format!(
                    "echo {}",
                    params["arguments"]["text"].as_str().unwrap_or("")
                )),
            }
        })
    });
    let tools: Arc<dyn Fn() -> Value + Send + Sync> = Arc::new(|| json!([{ "name": "Echo" }]));
    tokio::spawn(serve(
        BufReader::new(server_in),
        server_out,
        runner,
        tools,
        Arc::clone(&health),
        |_, _| Duration::from_millis(400),
    ));
    Rig {
        to_server,
        from_server: BufReader::new(from_server),
        health,
    }
}

impl Rig {
    async fn send(&mut self, v: Value) {
        self.to_server
            .write_all(format!("{v}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn call(&mut self, id: u64, tool: &str, text: &str) {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": tool, "arguments": { "text": text } } })).await;
    }

    async fn next(&mut self) -> Value {
        let mut line = String::new();
        self.from_server.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not json ({e}): {line:?}"))
    }

    fn text(v: &Value) -> String {
        v["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
}

#[tokio::test]
async fn a_call_that_never_returns_does_not_hold_up_another_tool() {
    let mut r = rig();
    r.call(1, "Hang", "").await;
    r.call(2, "Echo", "still here").await;
    let answer = tokio::time::timeout(Duration::from_millis(250), r.next())
        .await
        .expect("the second call waited behind the first");
    assert_eq!(answer["id"], 2);
    assert_eq!(Rig::text(&answer), "echo still here");
    assert_eq!(r.health.inflight(), 1, "the hung call is still counted");
    let (tool, _) = r.health.oldest_unanswered().unwrap();
    assert_eq!(tool, "Hang");
}

#[tokio::test]
async fn a_call_stuck_in_blocking_code_is_answered_for_and_does_not_hold_up_the_rest() {
    let mut r = rig();
    r.call(1, "Block", "").await;
    r.call(2, "Echo", "free").await;
    assert_eq!(
        Rig::text(
            &tokio::time::timeout(Duration::from_millis(250), r.next())
                .await
                .expect("echo waited")
        ),
        "echo free"
    );
    // The blocked call is answered with the deadline's error while its thread is still blocked.
    let gave_up = tokio::time::timeout(Duration::from_secs(1), r.next())
        .await
        .expect("the blocked call was never answered for");
    assert_eq!(gave_up["id"], 1);
    assert_eq!(gave_up["result"]["isError"], true);
    assert_eq!(
        r.health.call_threads(),
        1,
        "the stuck thread is still counted"
    );
    // A later call to the same tool is not stuck behind it for ever.
    r.call(3, "Block", "").await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), r.next())
            .await
            .expect("the next blocked call hung")["id"],
        3
    );
}

#[tokio::test]
async fn initialize_and_the_tool_list_are_answered_while_a_call_hangs() {
    let mut r = rig();
    r.call(1, "Hang", "").await;
    r.send(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .await;
    let list = tokio::time::timeout(Duration::from_millis(250), r.next())
        .await
        .expect("tools/list waited");
    assert_eq!(list["result"]["tools"][0]["name"], "Echo");
    r.send(json!({ "jsonrpc": "2.0", "id": 3, "method": "initialize", "params": {} }))
        .await;
    let init = tokio::time::timeout(Duration::from_millis(250), r.next())
        .await
        .expect("initialize waited");
    assert_eq!(init["result"]["serverInfo"]["name"], "agentmux-mcp");
}

#[tokio::test]
async fn a_call_that_never_returns_is_answered_with_an_error_before_the_cli_gives_up() {
    let mut r = rig();
    r.call(7, "Hang", "").await;
    let answer = tokio::time::timeout(Duration::from_secs(5), r.next())
        .await
        .expect("no answer for a call that never returns");
    assert_eq!(answer["id"], 7);
    assert_eq!(answer["result"]["isError"], true);
    assert!(
        Rig::text(&answer).contains("didn't finish within 0 s"),
        "{}",
        Rig::text(&answer)
    );
    assert_eq!(
        r.health.inflight(),
        0,
        "an answered call is no longer in flight"
    );
}

#[tokio::test]
async fn calls_to_one_tool_run_in_the_order_they_came() {
    let mut r = rig();
    r.call(1, "Slow", "").await;
    r.call(2, "Slow", "").await;
    r.call(3, "Echo", "x").await;
    // The echo isn't behind the slow ones; the slow ones answer in order.
    assert_eq!(r.next().await["id"], 3);
    let first = r.next().await;
    let second = r.next().await;
    assert_eq!(
        (first["id"].as_u64(), Rig::text(&first)),
        (Some(1), "slow 0".to_string())
    );
    assert_eq!(
        (second["id"].as_u64(), Rig::text(&second)),
        (Some(2), "slow 1".to_string())
    );
}

#[tokio::test]
async fn a_failed_or_panicking_call_is_an_error_answer_and_the_loop_goes_on() {
    let mut r = rig();
    r.call(1, "Fail", "").await;
    let failed = r.next().await;
    assert_eq!(
        (failed["result"]["isError"].clone(), Rig::text(&failed)),
        (json!(true), "it failed".to_string())
    );
    r.call(2, "Boom", "").await;
    let panicked = r.next().await;
    assert_eq!(panicked["result"]["isError"], true);
    assert!(
        Rig::text(&panicked).contains("stopped without an answer"),
        "{}",
        Rig::text(&panicked)
    );
    r.call(3, "Echo", "after").await;
    assert_eq!(Rig::text(&r.next().await), "echo after");
}

#[tokio::test]
async fn a_cancel_stops_the_call_and_frees_its_place() {
    let mut r = rig();
    r.call(1, "Hang", "").await;
    r.call(2, "Echo", "first").await;
    assert_eq!(r.next().await["id"], 2);
    assert_eq!(r.health.inflight(), 1);
    r.send(json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 1 } })).await;
    // A later call still works, and the cancelled one is no longer counted.
    r.call(3, "Echo", "second").await;
    assert_eq!(Rig::text(&r.next().await), "echo second");
    assert_eq!(r.health.inflight(), 0);
}

#[test]
fn deadlines_are_under_the_clis_idle_limit_and_longer_for_tools_that_wait() {
    let none = json!({});
    for tool in [
        "UI", "Terminal", "Browser", "Vault", "Widget", "Agents", "Message", "Desktop", "Memory",
        "Tasks", "Unknown",
    ] {
        assert!(
            call_deadline(tool, &none) < Duration::from_secs(1800),
            "{tool}"
        );
    }
    assert_eq!(call_deadline("UI", &none), Duration::from_secs(90));
    assert_eq!(call_deadline("Memory", &none), Duration::from_secs(300));
    // A shell wait gets its own timeout and a margin, but never more than 29 minutes.
    assert_eq!(
        call_deadline("Terminal", &json!({ "timeout_ms": 600_000 })),
        Duration::from_secs(660)
    );
    assert_eq!(
        call_deadline("Terminal", &json!({})),
        Duration::from_secs(150)
    );
    assert_eq!(
        call_deadline("Terminal", &json!({ "timeout_ms": 9_999_999_999u64 })),
        Duration::from_secs(29 * 60)
    );
}

#[test]
fn the_runtime_lag_grows_when_the_tick_stops() {
    let h = Health::new();
    h.tick();
    assert!(h.runtime_lag_ms() < 500);
    std::thread::sleep(Duration::from_millis(120));
    assert!(h.runtime_lag_ms() >= 100);
    h.tick();
    assert!(h.runtime_lag_ms() < 100);
}
