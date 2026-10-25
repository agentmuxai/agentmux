// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MCP request loop: JSON-RPC lines in, JSON-RPC lines out, with every
//! `tools/call` running as its own task under its own deadline, so one call
//! that never returns can't hold up the others
//! (docs/reports/ on #2614: a window call hung for 1800 s and every later call,
//! to any tool, hung behind it, for ten hours, until the agent restarted).
//!
//! Each call runs on a thread of its own (driving the same async runtime), so a
//! call stuck in blocking code, such as an OS window call into an app that has
//! stopped answering, holds that thread and nothing else.
//!
//! Calls to the same tool still run in the order they arrived (a `type` and a
//! `read` of the same shell mean something in order); calls to different tools
//! don't wait for each other. A call that outlives its deadline is answered
//! with an error and given up on (and so is a call that is cancelled): its
//! future is dropped, which stops async work at its next await. A call stuck in
//! blocking code can't be stopped, so its thread stays stuck, but the loop
//! doesn't, and its tool's queue moves on without it: the order holds among
//! the calls still being waited for.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

/// What a tool call returns: the text for the model, or the error to show it.
pub(crate) type CallResult = anyhow::Result<String>;
/// Not `Send`: a tool may hold an OS window handle across an await. The
/// future is made and run on one thread (see [`serve`]).
pub(crate) type CallFuture = Pin<Box<dyn Future<Output = CallResult>>>;

/// Runs one `tools/call`'s `params` (`{name, arguments}`).
pub(crate) trait ToolRunner: Send + Sync + 'static {
    fn run(&self, params: Value) -> CallFuture;
}

impl<F> ToolRunner for F
where
    F: Fn(Value) -> CallFuture + Send + Sync + 'static,
{
    fn run(&self, params: Value) -> CallFuture {
        self(params)
    }
}

/// Claude Code gives up on a call that is silent for 1800 s. A tool is
/// answered well before that.
const MAX_DEADLINE: Duration = Duration::from_secs(29 * 60);
const DEFAULT_DEADLINE: Duration = Duration::from_secs(5 * 60);
/// A window call (capture, look, click) takes seconds; a hung target window
/// makes the OS call wait for it forever.
const UI_DEADLINE: Duration = Duration::from_secs(90);

/// How long `tool` may run with `arguments` before the loop answers for it.
/// The tools that wait on purpose (a shell, the user, a spawned agent) get
/// their own wait, plus a margin; the rest get a few minutes.
pub(crate) fn call_deadline(tool: &str, arguments: &Value) -> Duration {
    let secs = |n: u64| Duration::from_secs(n);
    let wait_ms = |key: &str| arguments.get(key).and_then(Value::as_u64);
    let d = match tool {
        "UI" => UI_DEADLINE,
        // Waits on a pattern, the exit of a command or quiet, for `timeout_ms`.
        "Terminal" => {
            secs(150).max(Duration::from_millis(wait_ms("timeout_ms").unwrap_or(0)) + secs(60))
        }
        // A fill or click can wait for a page or the user to sign in.
        "Browser" | "Vault" | "Widget" => MAX_DEADLINE,
        // `spawn` waits for the new agent to start (360 s).
        "Agents" => secs(400),
        // File uploads (300 s).
        "Message" => secs(360),
        "Desktop" => secs(180),
        _ => DEFAULT_DEADLINE,
    };
    d.min(MAX_DEADLINE)
}

/// What the loop is doing, for the heartbeat to srv (`mcp_health`): the calls
/// not yet answered, and whether the async runtime is still turning.
pub(crate) struct Health {
    started: Instant,
    inflight: Mutex<HashMap<String, (String, Instant)>>,
    /// Milliseconds after `started` when the runtime last ran its tick task.
    last_tick_ms: AtomicU64,
    /// Call threads started and not yet back: a call given up on that is stuck
    /// in blocking code stays counted here.
    threads: AtomicU64,
}

impl Health {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            started: Instant::now(),
            inflight: Mutex::new(HashMap::new()),
            last_tick_ms: AtomicU64::new(0),
            threads: AtomicU64::new(0),
        })
    }

    fn threads_start(&self) {
        self.threads.fetch_add(1, Ordering::Relaxed);
    }

    fn threads_end(&self) {
        self.threads.fetch_sub(1, Ordering::Relaxed);
    }

    /// Call threads still running, answered for or not.
    pub(crate) fn call_threads(&self) -> u64 {
        self.threads.load(Ordering::Relaxed)
    }

    pub(crate) fn begin(&self, key: String, tool: String) {
        self.inflight
            .lock()
            .unwrap()
            .insert(key, (tool, Instant::now()));
    }

    fn end(&self, key: &str) {
        self.inflight.lock().unwrap().remove(key);
    }

    /// Calls received and not yet answered.
    pub(crate) fn inflight(&self) -> usize {
        self.inflight.lock().unwrap().len()
    }

    /// The age in seconds of the oldest call not yet answered, and its tool.
    pub(crate) fn oldest_unanswered(&self) -> Option<(String, u64)> {
        let map = self.inflight.lock().unwrap();
        map.values()
            .min_by_key(|(_, at)| *at)
            .map(|(tool, at)| (tool.clone(), at.elapsed().as_secs()))
    }

    /// Run on the runtime once a second: if it stops running, the runtime is
    /// starved or stuck, which [`Health::runtime_lag_ms`] shows from outside.
    pub(crate) fn tick(&self) {
        self.last_tick_ms
            .store(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    /// How long ago the runtime last ran [`Health::tick`], read from another thread.
    pub(crate) fn runtime_lag_ms(&self) -> u64 {
        (self.started.elapsed().as_millis() as u64)
            .saturating_sub(self.last_tick_ms.load(Ordering::Relaxed))
    }
}

fn error_result(id: &Value, text: String) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{"type": "text", "text": text}], "isError": true } })
}

fn text_result(id: &Value, text: String) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{"type": "text", "text": text}], "isError": false } })
}

/// One call thread in [`Health`]'s count, until it ends.
struct ThreadCount(Arc<Health>);

impl ThreadCount {
    fn start(health: Arc<Health>) -> Self {
        health.threads_start();
        Self(health)
    }
}

impl Drop for ThreadCount {
    fn drop(&mut self) {
        self.0.threads_end();
    }
}

/// The per-tool queues: calls to one tool run in arrival order.
#[derive(Default)]
struct ToolQueues(Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl ToolQueues {
    fn of(&self, tool: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(self.0.lock().unwrap().entry(tool.to_string()).or_default())
    }
}

/// Reads requests from `input` until it closes, answering each on `output`.
/// `deadline_of` says how long each call may run ([`call_deadline`] in production).
/// `initialize` and `tools/list` are answered at once; `tools/call` runs
/// through `runner` as a task. `notifications/cancelled` stops the call it names.
pub(crate) async fn serve<R, W>(
    mut input: R,
    output: W,
    runner: Arc<dyn ToolRunner>,
    tools: Arc<dyn Fn() -> Value + Send + Sync>,
    health: Arc<Health>,
    deadline_of: fn(&str, &Value) -> Duration,
) where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    // One writer, so two answers never interleave on the pipe.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(v) = out_rx.recv().await {
            let line = serde_json::to_string(&v).unwrap_or_default();
            if output.write_all(line.as_bytes()).await.is_err()
                || output.write_all(b"\n").await.is_err()
                || output.flush().await.is_err()
            {
                break;
            }
        }
    });
    let queues = Arc::new(ToolQueues::default());
    let running: Arc<Mutex<HashMap<String, tokio::task::AbortHandle>>> = Arc::default();
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        if id.is_null() {
            // A notification: no answer. Only a cancel does anything.
            if method == "notifications/cancelled" {
                if let Some(request) = params.get("requestId") {
                    if let Some(handle) = running.lock().unwrap().remove(&request.to_string()) {
                        handle.abort();
                        health.end(&request.to_string());
                    }
                }
            }
            continue;
        }
        match method {
            "initialize" => {
                let _ = out_tx.send(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "agentmux-mcp", "version": env!("CARGO_PKG_VERSION") }
                    }
                }));
            }
            "tools/list" => {
                let _ = out_tx
                    .send(json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools() } }));
            }
            "tools/call" => {
                let tool = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let deadline = deadline_of(&tool, params.get("arguments").unwrap_or(&Value::Null));
                let key = id.to_string();
                health.begin(key.clone(), tool.clone());
                let (runner, queue, out_tx, health, running_map) = (
                    Arc::clone(&runner),
                    queues.of(&tool),
                    out_tx.clone(),
                    Arc::clone(&health),
                    Arc::clone(&running),
                );
                let key_for_task = key.clone();
                let handle = tokio::runtime::Handle::current();
                // Held until the handle is in the map: the task removes its own
                // entry when it ends, and a fast call must not do that first.
                let mut running_now = running.lock().unwrap();
                let task = tokio::spawn(async move {
                    // The deadline covers the wait for the tool's earlier calls too.
                    let work = async {
                        let _turn = queue.lock().await;
                        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
                        // Dropped with `work` (the deadline, a cancel): the thread
                        // then drops the call's future instead of running it on.
                        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
                        let _cancel = cancel_tx;
                        let (runner, params) = (Arc::clone(&runner), params);
                        // Counts the thread until it ends, a panic included (or
                        // until the spawn fails and drops this with the closure).
                        let counted = ThreadCount::start(Arc::clone(&health));
                        let spawned = std::thread::Builder::new()
                            .name(format!("mcp-call-{tool}"))
                            .stack_size(4 << 20)
                            .spawn(move || {
                                let _counted = counted;
                                let result = handle.block_on(async {
                                    tokio::select! {
                                        result = runner.run(params) => Some(result),
                                        _ = cancel_rx => None,
                                    }
                                });
                                if let Some(result) = result {
                                    let _ = done_tx.send(result);
                                }
                            });
                        match spawned {
                            Ok(_) => done_rx
                                .await
                                .map_err(|_| "the call stopped without an answer".to_string()),
                            Err(e) => Err(format!("couldn't start a thread for the call: {e}")),
                        }
                    };
                    let answer = match tokio::time::timeout(deadline, work).await {
                        Ok(Ok(Ok(text))) => text_result(&id, text),
                        Ok(Ok(Err(e))) => error_result(&id, e.to_string()),
                        Ok(Err(why)) => error_result(&id, format!("{tool}: {why}")),
                        Err(_) => {
                            eprintln!(
                                "agentmux-mcp: {tool} did not finish within {} s; answered for it",
                                deadline.as_secs()
                            );
                            error_result(
                                &id,
                                format!(
                                    "{tool} didn't finish within {} s, so it was given up on and your other tools keep working. The work may still be running: check before you repeat it.",
                                    deadline.as_secs()
                                ),
                            )
                        }
                    };
                    health.end(&key_for_task);
                    running_map.lock().unwrap().remove(&key_for_task);
                    let _ = out_tx.send(answer);
                });
                running_now.insert(key, task.abort_handle());
                drop(running_now);
            }
            _ => {
                let _ = out_tx.send(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "method not found" } }));
            }
        }
    }
    drop(out_tx);
    // Answers already queued still go out.
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
}

#[cfg(test)]
#[path = "tests/serve_tests.rs"]
mod tests;
