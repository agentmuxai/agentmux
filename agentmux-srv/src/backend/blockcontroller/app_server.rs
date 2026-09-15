// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Provider-neutral App Server process and JSONL/JSON-RPC transport.
//!
//! This module deliberately stops at lifecycle and handshake. Thread, turn,
//! rendering, and interactive-request semantics live in the protocol adapter
//! added by later rollout slices. No provider selects this controller yet.

use std::collections::{HashMap, VecDeque};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot, watch, Mutex as AsyncMutex};

const INITIALIZING: u8 = 1;
const INITIALIZED: u8 = 2;
const INITIALIZE_FAILED: u8 = 3;

/// Conservative resource limits at the untrusted child-process boundary.
#[derive(Debug, Clone, Copy)]
pub struct AppServerLimits {
    pub max_frame_bytes: usize,
    pub max_pending_requests: usize,
    pub outbound_queue_capacity: usize,
    pub inbound_queue_capacity: usize,
    pub max_stderr_bytes: usize,
}

impl Default for AppServerLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 4 * 1024 * 1024,
            max_pending_requests: 128,
            outbound_queue_capacity: 128,
            inbound_queue_capacity: 256,
            max_stderr_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RpcId {
    Number(String),
    String(String),
    Null,
}

impl RpcId {
    fn from_value(value: &Value) -> Result<Self, AppServerError> {
        match value {
            Value::Number(number) if number.is_i64() || number.is_u64() => {
                Ok(Self::Number(number.to_string()))
            }
            Value::String(value) => Ok(Self::String(value.clone())),
            Value::Null => Ok(Self::Null),
            _ => Err(AppServerError::Protocol(
                "JSON-RPC id must be an integer, string, or null".to_string(),
            )),
        }
    }

    fn outgoing_u64(&self) -> Option<u64> {
        match self {
            Self::Number(value) => value.parse().ok(),
            Self::String(_) | Self::Null => None,
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Number(value) => serde_json::from_str(value).unwrap_or(Value::Null),
            Self::String(value) => Value::String(value.clone()),
            Self::Null => Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppServerIncoming {
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: RpcId,
        method: String,
        params: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum AppServerError {
    #[error("failed to spawn App Server: {0}")]
    Spawn(String),
    #[error("App Server I/O failed: {0}")]
    Io(String),
    #[error("App Server protocol violation: {0}")]
    Protocol(String),
    #[error("App Server request limit reached ({0})")]
    PendingRequestLimit(usize),
    #[error("App Server request timed out after {0:?}")]
    Timeout(Duration),
    #[error("App Server request was rejected ({code}): {message}")]
    Remote {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("App Server transport is closed")]
    Closed,
    #[error("App Server initialize may only be attempted once")]
    InitializeAlreadyAttempted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppServerExit {
    Clean {
        code: Option<i32>,
        stderr_tail: String,
    },
    ForcedAfterTimeout {
        stderr_tail: String,
    },
    BeforeInitialize {
        code: Option<i32>,
        stderr_tail: String,
    },
    NonZero {
        code: Option<i32>,
        stderr_tail: String,
    },
    ProtocolViolation {
        message: String,
        stderr_tail: String,
    },
    UnexpectedEof {
        stderr_tail: String,
    },
}

type PendingSender = oneshot::Sender<Result<Value, AppServerError>>;

struct SharedState {
    pending: Mutex<HashMap<u64, PendingSender>>,
    failure: Mutex<Option<AppServerError>>,
    closing: AtomicBool,
    closed: AtomicBool,
}

struct PendingGuard {
    shared: Arc<SharedState>,
    id: u64,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.shared.pending.lock().unwrap().remove(&self.id);
    }
}

enum Outbound {
    Frame {
        bytes: Vec<u8>,
        written: oneshot::Sender<Result<(), AppServerError>>,
    },
    Close {
        closed: oneshot::Sender<Result<(), AppServerError>>,
    },
}

pub struct AppServerTransport {
    outbound_tx: mpsc::Sender<Outbound>,
    incoming_rx: AsyncMutex<mpsc::Receiver<AppServerIncoming>>,
    reader_task: AsyncMutex<Option<tokio::task::JoinHandle<()>>>,
    shared: Arc<SharedState>,
    next_request_id: AtomicU64,
    initialize_state: AtomicU8,
    max_frame_bytes: usize,
    max_pending_requests: usize,
}

impl AppServerTransport {
    pub fn new<R, W>(reader: R, writer: W, limits: AppServerLimits) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let shared = Arc::new(SharedState {
            pending: Mutex::new(HashMap::new()),
            failure: Mutex::new(None),
            closing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        let (outbound_tx, outbound_rx) = mpsc::channel(limits.outbound_queue_capacity.max(1));
        let (incoming_tx, incoming_rx) = mpsc::channel(limits.inbound_queue_capacity.max(1));

        tokio::spawn(run_writer(writer, outbound_rx, shared.clone()));
        let reader_task = tokio::spawn(run_reader(
            reader,
            incoming_tx,
            shared.clone(),
            limits.max_frame_bytes,
        ));

        Self {
            outbound_tx,
            incoming_rx: AsyncMutex::new(incoming_rx),
            reader_task: AsyncMutex::new(Some(reader_task)),
            shared,
            next_request_id: AtomicU64::new(1),
            initialize_state: AtomicU8::new(0),
            max_frame_bytes: limits.max_frame_bytes,
            max_pending_requests: limits.max_pending_requests,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialize_state.load(Ordering::Acquire) == INITIALIZED
    }

    pub fn failure(&self) -> Option<AppServerError> {
        self.shared.failure.lock().unwrap().clone()
    }

    pub async fn next_incoming(&self) -> Option<AppServerIncoming> {
        self.incoming_rx.lock().await.recv().await
    }

    /// Wait for `run_reader` to finish processing whatever was already
    /// buffered on stdout. The reader terminates on its own shortly after
    /// the child's stdout pipe closes (EOF on process exit), but a caller
    /// classifying the exit right after `Child::wait()` returns can race
    /// ahead of it: `child.wait()` only reports the OS-level exit, not
    /// whether the reader has finished parsing the final bytes. Without
    /// this, a malformed/oversized final frame can be misclassified as
    /// `UnexpectedEof`/`NonZero`/`BeforeInitialize` instead of
    /// `ProtocolViolation`, because `self.failure()` hadn't been set yet.
    async fn wait_for_reader_to_finish(&self) {
        if let Some(task) = self.reader_task.lock().await.take() {
            let _ = task.await;
        }
    }

    pub async fn request(
        &self,
        method: &str,
        params: Value,
        request_timeout: Duration,
    ) -> Result<Value, AppServerError> {
        if let Some(error) = self.failure() {
            return Err(error);
        }
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(AppServerError::Closed);
        }

        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (response_tx, response_rx) = oneshot::channel();
        {
            let mut pending = self.shared.pending.lock().unwrap();
            if pending.len() >= self.max_pending_requests {
                return Err(AppServerError::PendingRequestLimit(
                    self.max_pending_requests,
                ));
            }
            pending.insert(id, response_tx);
        }
        let _pending_guard = PendingGuard {
            shared: self.shared.clone(),
            id,
        };

        let frame = serde_json::json!({
            "id": id,
            "method": method,
            "params": params,
        });

        let operation = async {
            self.write_value(frame).await?;
            response_rx.await.unwrap_or(Err(AppServerError::Closed))
        };

        match tokio::time::timeout(request_timeout, operation).await {
            Ok(result) => result,
            Err(_) => Err(AppServerError::Timeout(request_timeout)),
        }
    }

    pub async fn notify(
        &self,
        method: &str,
        params: Value,
        write_timeout: Duration,
    ) -> Result<(), AppServerError> {
        let mut frame = Map::from_iter([("method".to_string(), Value::String(method.to_string()))]);
        if !params.is_null() {
            frame.insert("params".to_string(), params);
        }
        tokio::time::timeout(write_timeout, self.write_value(Value::Object(frame)))
            .await
            .map_err(|_| AppServerError::Timeout(write_timeout))?
    }

    /// Reply to a server-initiated request. The caller must have an explicit
    /// policy/UI decision; this primitive never invents an approval.
    pub async fn respond(
        &self,
        id: RpcId,
        result: Result<Value, ServerResponseError>,
        write_timeout: Duration,
    ) -> Result<(), AppServerError> {
        self.respond_tracked(id, result, write_timeout, &WriteOutcomeTracker::new())
            .await
    }

    /// Same as `respond`, but also reports via `outcome` whether it would
    /// be safe to retry after a non-success result — including a caller
    /// whose own future is cancelled while this is still awaiting the
    /// write-confirmation ack, which never produces a return value at all
    /// (see `WriteOutcomeTracker`, `write_value_tracked`). A caller that
    /// mistakes "timed out/cancelled waiting for confirmation" for "never
    /// sent" and retries risks a second, duplicate response for the same
    /// id (ReAgent P1, PR #3212 6th review).
    pub async fn respond_tracked(
        &self,
        id: RpcId,
        result: Result<Value, ServerResponseError>,
        write_timeout: Duration,
        outcome: &WriteOutcomeTracker,
    ) -> Result<(), AppServerError> {
        let mut frame = Map::new();
        frame.insert("id".to_string(), id.to_value());
        match result {
            Ok(result) => {
                frame.insert("result".to_string(), result);
            }
            Err(error) => {
                frame.insert(
                    "error".to_string(),
                    serde_json::to_value(error)
                        .map_err(|error| AppServerError::Protocol(error.to_string()))?,
                );
            }
        }
        tokio::time::timeout(
            write_timeout,
            self.write_value_tracked(Value::Object(frame), outcome),
        )
        .await
        .map_err(|_| AppServerError::Timeout(write_timeout))?
    }

    pub async fn initialize(&self, timeout: Duration) -> Result<Value, AppServerError> {
        if self
            .initialize_state
            .compare_exchange(0, INITIALIZING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(AppServerError::InitializeAlreadyAttempted);
        }

        let params = InitializeParams {
            client_info: ClientInfo {
                name: "agentmux",
                title: Some("AgentMux"),
                version: env!("CARGO_PKG_VERSION"),
            },
            capabilities: InitializeCapabilities {
                experimental_api: false,
            },
        };
        let params = serde_json::to_value(params)
            .map_err(|error| AppServerError::Protocol(error.to_string()))?;

        let result = match self.request("initialize", params, timeout).await {
            Ok(result) => result,
            Err(error) => {
                self.initialize_state
                    .store(INITIALIZE_FAILED, Ordering::Release);
                return Err(error);
            }
        };
        if let Err(error) = self.notify("initialized", Value::Null, timeout).await {
            self.initialize_state
                .store(INITIALIZE_FAILED, Ordering::Release);
            return Err(error);
        }
        self.initialize_state.store(INITIALIZED, Ordering::Release);
        Ok(result)
    }

    pub async fn shutdown(&self, timeout: Duration) -> Result<(), AppServerError> {
        self.shared.closing.store(true, Ordering::Release);
        let operation = async {
            let (closed_tx, closed_rx) = oneshot::channel();
            self.outbound_tx
                .send(Outbound::Close { closed: closed_tx })
                .await
                .map_err(|_| AppServerError::Closed)?;
            closed_rx.await.unwrap_or(Err(AppServerError::Closed))
        };
        let result = tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| AppServerError::Timeout(timeout))?;
        fail_transport(&self.shared, AppServerError::Closed);
        result
    }

    async fn write_value(&self, value: Value) -> Result<(), AppServerError> {
        self.write_value_tracked(value, &WriteOutcomeTracker::new())
            .await
    }

    /// See `WriteOutcomeTracker`. `outcome` moves to `OutcomeUnconfirmed`
    /// the instant the frame is handed to `run_writer` — before awaiting
    /// its write-confirmation ack — and to `ConfirmedUndelivered` if that
    /// ack later reports a genuine failure, rather than staying stuck at
    /// "unconfirmed" forever for an outcome that in fact became known.
    async fn write_value_tracked(
        &self,
        value: Value,
        outcome: &WriteOutcomeTracker,
    ) -> Result<(), AppServerError> {
        if let Some(error) = self.failure() {
            return Err(error);
        }
        if self.shared.closing.load(Ordering::Acquire) || self.shared.closed.load(Ordering::Acquire)
        {
            return Err(AppServerError::Closed);
        }
        let mut bytes = serde_json::to_vec(&value)
            .map_err(|error| AppServerError::Protocol(error.to_string()))?;
        if bytes.len() > self.max_frame_bytes {
            return Err(AppServerError::Protocol(format!(
                "outbound frame is {} bytes; limit is {}",
                bytes.len(),
                self.max_frame_bytes
            )));
        }
        bytes.push(b'\n');
        let (written_tx, written_rx) = oneshot::channel();
        self.outbound_tx
            .send(Outbound::Frame {
                bytes,
                written: written_tx,
            })
            .await
            .map_err(|_| AppServerError::Closed)?;
        // From here on the frame is irrevocably queued for writing by
        // run_writer, independent of `written_rx` — dropping this future
        // (a timeout or cancellation) cannot un-send it, so from this
        // point a caller must not treat "didn't observe a confirmed
        // outcome" the same as "never sent."
        outcome.mark_queued();
        let result = written_rx.await.unwrap_or(Err(AppServerError::Closed));
        if result.is_err() {
            // The writer itself confirmed delivery failed — genuinely
            // never sent after all, so it's safe to retry again.
            outcome.mark_confirmed_undelivered();
        }
        result
    }
}

/// Tracks, for a single tracked outbound write, whether retrying after a
/// non-success outcome risks sending the same frame twice. A plain
/// success/failure `Result` isn't enough for this: once a frame is handed
/// to `run_writer` it will be delivered independent of anything that
/// happens afterward to the caller's future, including that future being
/// dropped/cancelled before it ever observes an outcome — a case a Result
/// alone can't represent, but which a caller's own `Drop` glue can still
/// consult here (ReAgent P1, PR #3212 6th review).
#[derive(Debug)]
pub struct WriteOutcomeTracker(AtomicU8);

impl WriteOutcomeTracker {
    const NOT_QUEUED: u8 = 0;
    const OUTCOME_UNCONFIRMED: u8 = 1;
    const CONFIRMED_UNDELIVERED: u8 = 2;

    pub fn new() -> Self {
        Self(AtomicU8::new(Self::NOT_QUEUED))
    }

    fn mark_queued(&self) {
        self.0.store(Self::OUTCOME_UNCONFIRMED, Ordering::Release);
    }

    fn mark_confirmed_undelivered(&self) {
        self.0
            .store(Self::CONFIRMED_UNDELIVERED, Ordering::Release);
    }

    /// True if the frame was never queued, or was queued and its delivery
    /// is now confirmed to have failed — in both cases nothing was, or
    /// ever will be, delivered, so sending again cannot produce a
    /// duplicate. False once queued but unconfirmed, which also covers a
    /// confirmed success (obviously never safe to retry either).
    pub fn safe_to_retry(&self) -> bool {
        matches!(
            self.0.load(Ordering::Acquire),
            Self::NOT_QUEUED | Self::CONFIRMED_UNDELIVERED
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerResponseError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InitializeParams<'a> {
    client_info: ClientInfo<'a>,
    capabilities: InitializeCapabilities,
}

#[derive(Serialize)]
struct ClientInfo<'a> {
    name: &'a str,
    title: Option<&'a str>,
    version: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InitializeCapabilities {
    experimental_api: bool,
}

async fn run_writer<W>(
    mut writer: W,
    mut outbound_rx: mpsc::Receiver<Outbound>,
    shared: Arc<SharedState>,
) where
    W: AsyncWrite + Unpin,
{
    while let Some(outbound) = outbound_rx.recv().await {
        match outbound {
            Outbound::Frame { bytes, written } => {
                let result = async {
                    writer.write_all(&bytes).await?;
                    writer.flush().await
                }
                .await
                .map_err(|error: std::io::Error| AppServerError::Io(error.to_string()));
                let _ = written.send(result.clone());
                if let Err(error) = result {
                    fail_transport(&shared, error);
                    break;
                }
            }
            Outbound::Close { closed } => {
                let result = writer
                    .shutdown()
                    .await
                    .map_err(|error| AppServerError::Io(error.to_string()));
                let _ = closed.send(result.clone());
                if let Err(error) = result {
                    fail_transport(&shared, error);
                }
                break;
            }
        }
    }
    shared.closed.store(true, Ordering::Release);
}

async fn run_reader<R>(
    reader: R,
    incoming_tx: mpsc::Sender<AppServerIncoming>,
    shared: Arc<SharedState>,
    max_frame_bytes: usize,
) where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    loop {
        let frame = match read_frame(&mut reader, max_frame_bytes).await {
            Ok(Some(frame)) => frame,
            Ok(None) if shared.closing.load(Ordering::Acquire) => break,
            Ok(None) => {
                fail_transport(&shared, AppServerError::Closed);
                break;
            }
            Err(error) => {
                fail_transport(&shared, error);
                break;
            }
        };

        match classify_frame(&frame) {
            Ok(WireMessage::Response { id, result }) => {
                let Some(id) = id.outgoing_u64() else {
                    tracing::warn!(?id, "ignoring App Server response with non-client id");
                    continue;
                };
                let pending = shared.pending.lock().unwrap().remove(&id);
                if let Some(pending) = pending {
                    let _ = pending.send(result);
                } else {
                    tracing::warn!(
                        id,
                        "ignoring duplicate, unknown, or cancelled App Server response"
                    );
                }
            }
            Ok(WireMessage::Incoming(incoming)) => {
                if incoming_tx.try_send(incoming).is_err() {
                    fail_transport(
                        &shared,
                        AppServerError::Protocol(
                            "inbound App Server event queue exceeded its bound".to_string(),
                        ),
                    );
                    break;
                }
            }
            Err(error) => {
                fail_transport(&shared, error);
                break;
            }
        }
    }
}

async fn read_frame<R>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> Result<Option<Vec<u8>>, AppServerError>
where
    R: AsyncBufRead + Unpin,
{
    let mut frame = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|error| AppServerError::Io(error.to_string()))?;
        if available.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Ok(Some(frame))
            };
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let payload_bytes = newline.map_or(consumed, |index| index);
        if frame.len().saturating_add(payload_bytes) > max_frame_bytes {
            return Err(AppServerError::Protocol(format!(
                "inbound frame exceeds {max_frame_bytes} bytes"
            )));
        }
        frame.extend_from_slice(&available[..payload_bytes]);
        reader.consume(consumed);

        if newline.is_some() {
            if frame.last() == Some(&b'\r') {
                frame.pop();
            }
            return Ok(Some(frame));
        }
    }
}

enum WireMessage {
    Response {
        id: RpcId,
        result: Result<Value, AppServerError>,
    },
    Incoming(AppServerIncoming),
}

fn classify_frame(frame: &[u8]) -> Result<WireMessage, AppServerError> {
    if frame.is_empty() {
        return Err(AppServerError::Protocol(
            "stdout contained an empty frame".to_string(),
        ));
    }
    let value: Value = serde_json::from_slice(frame)
        .map_err(|error| AppServerError::Protocol(format!("invalid JSON frame: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| AppServerError::Protocol("JSON-RPC frame must be an object".to_string()))?;
    validate_jsonrpc_marker(object)?;

    if let Some(method) = object.get("method") {
        let method = method.as_str().ok_or_else(|| {
            AppServerError::Protocol("JSON-RPC method must be a string".to_string())
        })?;
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        return if let Some(id) = object.get("id") {
            Ok(WireMessage::Incoming(AppServerIncoming::Request {
                id: RpcId::from_value(id)?,
                method: method.to_string(),
                params,
            }))
        } else {
            Ok(WireMessage::Incoming(AppServerIncoming::Notification {
                method: method.to_string(),
                params,
            }))
        };
    }

    let id = object
        .get("id")
        .ok_or_else(|| AppServerError::Protocol("response is missing an id".to_string()))?;
    let id = RpcId::from_value(id)?;
    let has_result = object.contains_key("result");
    let has_error = object.contains_key("error");
    if has_result == has_error {
        return Err(AppServerError::Protocol(
            "response must contain exactly one of result or error".to_string(),
        ));
    }
    let result = if has_result {
        Ok(object.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(parse_remote_error(object.get("error").unwrap())?)
    };
    Ok(WireMessage::Response { id, result })
}

fn validate_jsonrpc_marker(object: &Map<String, Value>) -> Result<(), AppServerError> {
    match object.get("jsonrpc") {
        None => Ok(()), // Codex App Server intentionally omits the JSON-RPC header.
        Some(Value::String(version)) if version == "2.0" => Ok(()),
        Some(_) => Err(AppServerError::Protocol(
            "jsonrpc, when present, must equal \"2.0\"".to_string(),
        )),
    }
}

fn parse_remote_error(value: &Value) -> Result<AppServerError, AppServerError> {
    let error = value
        .as_object()
        .ok_or_else(|| AppServerError::Protocol("response error must be an object".to_string()))?;
    let code = error.get("code").and_then(Value::as_i64).ok_or_else(|| {
        AppServerError::Protocol("response error code must be an integer".to_string())
    })?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppServerError::Protocol("response error message must be a string".to_string())
        })?;
    Ok(AppServerError::Remote {
        code,
        message: message.to_string(),
        data: error.get("data").cloned(),
    })
}

fn fail_transport(shared: &SharedState, error: AppServerError) {
    let failure = {
        let mut failure = shared.failure.lock().unwrap();
        if failure.is_none() {
            *failure = Some(error.clone());
        }
        failure.clone().unwrap()
    };
    shared.closed.store(true, Ordering::Release);
    let pending = std::mem::take(&mut *shared.pending.lock().unwrap());
    for (_, sender) in pending {
        let _ = sender.send(Err(failure.clone()));
    }
}

struct StderrTail {
    bytes: VecDeque<u8>,
    max_bytes: usize,
}

impl StderrTail {
    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend(chunk);
        while self.bytes.len() > self.max_bytes {
            self.bytes.pop_front();
        }
    }

    fn snapshot(&self) -> String {
        let bytes: Vec<u8> = self.bytes.iter().copied().collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

pub struct AppServerProcess {
    pub transport: AppServerTransport,
    // A single dedicated reaper task (spawned once, below) owns `Child`
    // exclusively for its whole lifetime and is the only thing that ever
    // calls `child.wait()`. This exists because `wait_for_exit()` and
    // `shutdown()` can legitimately run concurrently (one waiting
    // passively for natural exit, the other trying to force-stop) and both
    // need to observe the same exit — but `Child::wait()`/`Child::kill()`
    // both require `&mut Child`, so whichever call took the lock first
    // used to block the other for as long as the child kept running,
    // including `shutdown()`'s own force-kill step (ReAgent P1, PR #3209).
    // `watch` (not `Notify`/`oneshot`) specifically because it's safe for
    // any number of callers to subscribe at any time, including after the
    // child has already exited — a late subscriber just sees the value
    // that's already there instead of missing a wakeup that already fired.
    exit_status: watch::Receiver<Option<Result<ExitStatus, String>>>,
    stderr_tail: Arc<Mutex<StderrTail>>,
    stderr_task: AsyncMutex<Option<tokio::task::JoinHandle<()>>>,
    shutdown_requested: AtomicBool,
    pid: Option<u32>,
}

impl AppServerProcess {
    pub fn spawn(mut command: Command, limits: AppServerLimits) -> Result<Self, AppServerError> {
        command.kill_on_drop(true);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| AppServerError::Spawn(error.to_string()))?;
        let pid = child.id();
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppServerError::Spawn("stdin was not captured".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppServerError::Spawn("stdout was not captured".to_string()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppServerError::Spawn("stderr was not captured".to_string()))?;

        let stderr_tail = Arc::new(Mutex::new(StderrTail {
            bytes: VecDeque::new(),
            max_bytes: limits.max_stderr_bytes,
        }));
        let stderr_tail_task = stderr_tail.clone();
        let stderr_task = tokio::spawn(async move {
            let mut buffer = [0_u8; 4096];
            loop {
                match stderr.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(read) => stderr_tail_task.lock().unwrap().push(&buffer[..read]),
                    Err(error) => {
                        tracing::warn!(%error, "App Server stderr read failed");
                        break;
                    }
                }
            }
        });

        // Both stdout and stderr readers exist before initialize can write.
        let transport = AppServerTransport::new(stdout, stdin, limits);

        let (exit_tx, exit_rx) = watch::channel(None);
        tokio::spawn(async move {
            let result = child
                .wait()
                .await
                .map_err(|error| error.to_string());
            // `child` (and its `kill_on_drop`) drops here, after it has
            // already exited — a no-op, not a race with anyone else, since
            // this task is the only place that ever touches `Child`.
            let _ = exit_tx.send(Some(result));
        });

        Ok(Self {
            transport,
            exit_status: exit_rx,
            stderr_tail,
            stderr_task: AsyncMutex::new(Some(stderr_task)),
            shutdown_requested: AtomicBool::new(false),
            pid,
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub async fn spawn_initialized(
        command: Command,
        limits: AppServerLimits,
        initialize_timeout: Duration,
    ) -> Result<Self, AppServerError> {
        let process = Self::spawn(command, limits)?;
        if let Err(error) = process.transport.initialize(initialize_timeout).await {
            let _ = process.shutdown(Duration::from_secs(1)).await;
            return Err(error);
        }
        Ok(process)
    }

    /// Await the reaper task's exit status. Race-free for any number of
    /// concurrent/late callers — see `exit_status`'s doc comment on why
    /// this exists instead of each caller calling `Child::wait()` itself.
    async fn await_exit(&self) -> Result<ExitStatus, AppServerError> {
        let mut rx = self.exit_status.clone();
        loop {
            if let Some(result) = rx.borrow().clone() {
                return result.map_err(AppServerError::Io);
            }
            if rx.changed().await.is_err() {
                return Err(AppServerError::Closed);
            }
        }
    }

    pub async fn wait_for_exit(&self) -> Result<AppServerExit, AppServerError> {
        let status = self.await_exit().await?;
        self.transport.wait_for_reader_to_finish().await;
        self.finish_stderr().await;
        Ok(self.classify_exit(status))
    }

    pub async fn shutdown(&self, timeout: Duration) -> Result<AppServerExit, AppServerError> {
        self.shutdown_requested.store(true, Ordering::Release);
        let _ = self.transport.shutdown(timeout).await;
        let status = match tokio::time::timeout(timeout, self.await_exit()).await {
            Ok(result) => Some(result?),
            Err(_) => {
                // Force-kill by pid rather than `Child::kill()` — the
                // reaper task spawned in `spawn()` owns the `Child` value
                // exclusively for its whole lifetime (that's what fixes the
                // concurrent-shutdown deadlock this replaces), so nothing
                // outside that task can get `&mut Child` to call `.kill()`
                // on it. `kill_pid` needs only the raw pid, not the handle.
                if let Some(pid) = self.pid {
                    let _ = agentmux_common::process::kill_pid(pid);
                }
                // The kill above makes the reaper's own wait() return
                // shortly; discard the resulting status like the pre-fix
                // code did (`ForcedAfterTimeout` below reports the timeout,
                // not whatever exit code a killed process happens to have).
                let _ = self.await_exit().await;
                None
            }
        };
        self.transport.wait_for_reader_to_finish().await;
        self.finish_stderr().await;
        match status {
            Some(status) => Ok(self.classify_exit(status)),
            None => Ok(AppServerExit::ForcedAfterTimeout {
                stderr_tail: self.stderr_snapshot(),
            }),
        }
    }

    async fn finish_stderr(&self) {
        if let Some(task) = self.stderr_task.lock().await.take() {
            let _ = task.await;
        }
    }

    fn stderr_snapshot(&self) -> String {
        self.stderr_tail.lock().unwrap().snapshot()
    }

    fn classify_exit(&self, status: ExitStatus) -> AppServerExit {
        let stderr_tail = self.stderr_snapshot();
        if let Some(AppServerError::Protocol(message)) = self.transport.failure() {
            return AppServerExit::ProtocolViolation {
                message,
                stderr_tail,
            };
        }
        if !self.transport.is_initialized() {
            return AppServerExit::BeforeInitialize {
                code: status.code(),
                stderr_tail,
            };
        }
        if !status.success() {
            return AppServerExit::NonZero {
                code: status.code(),
                stderr_tail,
            };
        }
        if self.shutdown_requested.load(Ordering::Acquire) {
            AppServerExit::Clean {
                code: status.code(),
                stderr_tail,
            }
        } else {
            AppServerExit::UnexpectedEof { stderr_tail }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use serde_json::json;
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;

    #[tokio::test]
    async fn correlates_out_of_order_responses_and_preserves_notifications() {
        let (client_io, server_io) = duplex(8192);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let transport = Arc::new(AppServerTransport::new(
            client_reader,
            client_writer,
            AppServerLimits::default(),
        ));

        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let first: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            let second: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            let (first_id, second_id) = if first["method"] == "one" {
                (&first["id"], &second["id"])
            } else {
                (&second["id"], &first["id"])
            };
            let frames = format!(
                "{{\"method\":\"thread/status/changed\",\"params\":{{\"ok\":true}}}}\r\n{{\"id\":{},\"result\":\"second\"}}\n{{\"id\":{},\"result\":\"first\"}}\n",
                second_id, first_id
            );
            server_writer
                .write_all(&frames.as_bytes()[..17])
                .await
                .unwrap();
            server_writer
                .write_all(&frames.as_bytes()[17..])
                .await
                .unwrap();
        });

        let first = {
            let transport = transport.clone();
            tokio::spawn(async move {
                transport
                    .request("one", Value::Null, Duration::from_secs(1))
                    .await
            })
        };
        let second = {
            let transport = transport.clone();
            tokio::spawn(async move {
                transport
                    .request("two", Value::Null, Duration::from_secs(1))
                    .await
            })
        };

        assert_eq!(first.await.unwrap().unwrap(), Value::String("first".into()));
        assert_eq!(
            second.await.unwrap().unwrap(),
            Value::String("second".into())
        );
        assert_eq!(
            transport.next_incoming().await,
            Some(AppServerIncoming::Notification {
                method: "thread/status/changed".to_string(),
                params: serde_json::json!({ "ok": true }),
            })
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn remote_errors_resolve_only_the_matching_request() {
        let (client_io, server_io) = duplex(4096);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let transport =
            AppServerTransport::new(client_reader, client_writer, AppServerLimits::default());
        let server = tokio::spawn(async move {
            let request = BufReader::new(server_reader)
                .lines()
                .next_line()
                .await
                .unwrap()
                .unwrap();
            let request: Value = serde_json::from_str(&request).unwrap();
            server_writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"error\":{{\"code\":-32001,\"message\":\"no thread\",\"data\":{{\"retry\":false}}}}}}\n",
                        request["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        assert_eq!(
            transport
                .request("thread/resume", Value::Null, Duration::from_secs(1))
                .await,
            Err(AppServerError::Remote {
                code: -32001,
                message: "no thread".to_string(),
                data: Some(serde_json::json!({ "retry": false })),
            })
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn responds_to_server_requests_without_auto_approving_them() {
        let (client_io, server_io) = duplex(4096);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let transport =
            AppServerTransport::new(client_reader, client_writer, AppServerLimits::default());
        server_writer
            .write_all(
                br#"{"id":"approval-1","method":"item/commandExecution/requestApproval","params":{"command":"echo hi"}}
"#,
            )
            .await
            .unwrap();
        let request = transport.next_incoming().await.unwrap();
        let AppServerIncoming::Request { id, method, .. } = request else {
            panic!("expected server request");
        };
        assert_eq!(method, "item/commandExecution/requestApproval");
        transport
            .respond(id, Ok(json!({"decision":"accept"})), Duration::from_secs(1))
            .await
            .unwrap();
        let response = BufReader::new(server_reader)
            .lines()
            .next_line()
            .await
            .unwrap()
            .unwrap();
        let response: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(
            response,
            json!({"id":"approval-1","result":{"decision":"accept"}})
        );
    }

    #[test]
    fn preserves_string_numeric_and_null_server_request_ids() {
        let cases = [
            (
                br#"{"id":"approval-7","method":"item/commandExecution/requestApproval"}"#
                    .as_slice(),
                RpcId::String("approval-7".to_string()),
            ),
            (
                br#"{"id":42,"method":"item/fileChange/requestApproval"}"#.as_slice(),
                RpcId::Number("42".to_string()),
            ),
            (
                br#"{"id":null,"method":"tool/requestUserInput"}"#.as_slice(),
                RpcId::Null,
            ),
        ];
        for (frame, expected_id) in cases {
            match classify_frame(frame).unwrap() {
                WireMessage::Incoming(AppServerIncoming::Request { id, .. }) => {
                    assert_eq!(id, expected_id)
                }
                _ => panic!("expected a server request"),
            }
        }

        assert!(matches!(
            classify_frame(br#"{"result":{}}"#),
            Err(AppServerError::Protocol(message)) if message.contains("missing an id")
        ));
        assert!(matches!(
            classify_frame(br#"{"id":1,"result":{},"error":{"code":-1,"message":"bad"}}"#),
            Err(AppServerError::Protocol(message)) if message.contains("exactly one")
        ));
    }

    #[tokio::test]
    async fn bounds_pending_requests_and_releases_cancelled_slots() {
        let limits = AppServerLimits {
            max_pending_requests: 1,
            ..AppServerLimits::default()
        };
        let (client_io, server_io) = duplex(4096);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let (server_reader, _server_writer) = tokio::io::split(server_io);
        let transport = Arc::new(AppServerTransport::new(
            client_reader,
            client_writer,
            limits,
        ));
        let (observed_tx, observed_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let _ = lines.next_line().await.unwrap().unwrap();
            let _ = observed_tx.send(());
            while lines.next_line().await.unwrap().is_some() {}
        });
        let first = {
            let transport = transport.clone();
            tokio::spawn(async move {
                transport
                    .request("one", Value::Null, Duration::from_secs(5))
                    .await
            })
        };
        observed_rx.await.unwrap();

        assert_eq!(
            transport
                .request("two", Value::Null, Duration::from_secs(1))
                .await,
            Err(AppServerError::PendingRequestLimit(1))
        );
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        assert!(transport.shared.pending.lock().unwrap().is_empty());

        assert_eq!(
            transport
                .request("timeout", Value::Null, Duration::from_millis(20))
                .await,
            Err(AppServerError::Timeout(Duration::from_millis(20)))
        );
        assert!(transport.shared.pending.lock().unwrap().is_empty());
        server.abort();
    }

    #[tokio::test]
    async fn malformed_and_oversized_frames_fail_closed() {
        let limits = AppServerLimits {
            max_frame_bytes: 16,
            ..AppServerLimits::default()
        };
        let (client_io, mut server_io) = duplex(4096);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let transport = AppServerTransport::new(client_reader, client_writer, limits);
        server_io.write_all(b"not-json\n").await.unwrap();
        assert_eq!(transport.next_incoming().await, None);
        let error = transport
            .request("after-error", Value::Null, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(error, AppServerError::Protocol(_)));

        let (client_io, mut server_io) = duplex(4096);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let transport = AppServerTransport::new(client_reader, client_writer, limits);
        server_io
            .write_all(b"{\"way\":\"too long for bound\"}\n")
            .await
            .unwrap();
        assert_eq!(transport.next_incoming().await, None);
        let error = transport
            .request("after-error", Value::Null, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(error, AppServerError::Protocol(_)));
    }

    #[tokio::test]
    async fn initializes_and_gracefully_stops_a_fake_process() {
        let process = AppServerProcess::spawn(fake_command("happy"), AppServerLimits::default())
            .expect("spawn fake App Server");
        let response = process
            .transport
            .initialize(Duration::from_secs(2))
            .await
            .expect("initialize");
        assert_eq!(response["serverInfo"]["name"], "fake-app-server");
        assert!(process.transport.is_initialized());
        assert_eq!(
            process.transport.next_incoming().await,
            Some(AppServerIncoming::Notification {
                method: "server/ready".to_string(),
                params: serde_json::json!({ "fixture": true }),
            })
        );

        let exit = process.shutdown(Duration::from_secs(2)).await.unwrap();
        match exit {
            AppServerExit::Clean { stderr_tail, .. } => {
                assert!(stderr_tail.contains("fake server started"));
                assert!(stderr_tail.contains("fake server observed EOF"));
            }
            other => panic!("expected clean exit, got {other:?}"),
        }
    }

    /// ReAgent P1, PR #3209: `wait_for_exit()` used to hold the `AsyncMutex`
    /// guarding `child` for the entire duration of `Child::wait()`, so a
    /// concurrent `shutdown()` (e.g. spawn_event_loop's passive EOF-wait
    /// racing a caller-initiated stop) could never acquire that same lock
    /// to run its own force-kill step — it would just block on the lock
    /// forever, defeating shutdown's own timeout entirely. Every assertion
    /// here is wrapped in a bounded `tokio::time::timeout` specifically so
    /// a regression fails this test in seconds instead of hanging the
    /// whole suite.
    #[tokio::test]
    async fn shutdown_can_force_kill_while_wait_for_exit_is_concurrently_awaiting() {
        let process = Arc::new(
            AppServerProcess::spawn(fake_command("ignores-shutdown"), AppServerLimits::default())
                .expect("spawn fake App Server"),
        );
        process
            .transport
            .initialize(Duration::from_secs(2))
            .await
            .expect("initialize");

        // Starts waiting BEFORE shutdown() is called, mirroring the real
        // race: something else already passively waiting when a stop comes in.
        let waiter_process = process.clone();
        let waiter = tokio::spawn(async move { waiter_process.wait_for_exit().await });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let shutdown_result = tokio::time::timeout(
            Duration::from_secs(5),
            process.shutdown(Duration::from_millis(200)),
        )
        .await
        .expect("shutdown() must not hang waiting on the same lock wait_for_exit() holds")
        .unwrap();
        assert!(
            matches!(shutdown_result, AppServerExit::ForcedAfterTimeout { .. }),
            "expected a forced kill after the 200ms grace period (this fixture ignores \
             graceful shutdown by design), got {shutdown_result:?}"
        );

        // The waiter and shutdown() don't necessarily agree on the exit
        // CLASSIFICATION — shutdown() knows it forced the kill and reports
        // that deliberately (discarding the real exit code); wait_for_exit()
        // is a passive observer that just sees the killed process's actual
        // (non-zero) exit code and classifies it normally. What matters
        // here is that it resolves at all, promptly, instead of hanging —
        // that's the deadlock this fix removes.
        let waiter_result = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("the concurrent wait_for_exit() must resolve once shutdown() kills the process")
            .unwrap()
            .unwrap();
        assert!(
            matches!(waiter_result, AppServerExit::NonZero { .. }),
            "expected the killed process's real non-zero exit to surface here, got {waiter_result:?}"
        );
    }

    #[tokio::test]
    async fn classifies_exit_before_initialize_with_bounded_stderr() {
        let limits = AppServerLimits {
            max_stderr_bytes: 24,
            ..AppServerLimits::default()
        };
        let process = AppServerProcess::spawn(fake_command("exit-before-init"), limits)
            .expect("spawn fake App Server");
        let exit = process.wait_for_exit().await.unwrap();
        match exit {
            AppServerExit::BeforeInitialize { code, stderr_tail } => {
                assert_eq!(code, Some(23));
                assert!(stderr_tail.len() <= 24);
                assert!(stderr_tail.ends_with("before initialize\n"));
            }
            other => panic!("expected pre-initialize exit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn classifies_post_initialize_nonzero_exit_and_unexpected_eof() {
        let process =
            AppServerProcess::spawn(fake_command("exit-after-init"), AppServerLimits::default())
                .expect("spawn fake App Server");
        process
            .transport
            .initialize(Duration::from_secs(2))
            .await
            .unwrap();
        match process.wait_for_exit().await.unwrap() {
            AppServerExit::NonZero { code, stderr_tail } => {
                assert_eq!(code, Some(27));
                assert!(stderr_tail.contains("failed after initialize"));
            }
            other => panic!("expected post-initialize failure, got {other:?}"),
        }

        let process =
            AppServerProcess::spawn(fake_command("eof-after-init"), AppServerLimits::default())
                .expect("spawn fake App Server");
        process
            .transport
            .initialize(Duration::from_secs(2))
            .await
            .unwrap();
        assert!(matches!(
            process.wait_for_exit().await.unwrap(),
            AppServerExit::UnexpectedEof { .. }
        ));
    }

    #[tokio::test]
    async fn classifies_malformed_stdout_as_protocol_violation() {
        let process = AppServerProcess::spawn(
            fake_command("protocol-violation"),
            AppServerLimits::default(),
        )
        .expect("spawn fake App Server");
        assert!(matches!(
            process.transport.initialize(Duration::from_secs(2)).await,
            Err(AppServerError::Protocol(_))
        ));
        match process.shutdown(Duration::from_secs(2)).await.unwrap() {
            AppServerExit::ProtocolViolation {
                message,
                stderr_tail,
            } => {
                assert!(message.contains("invalid JSON"));
                assert!(stderr_tail.contains("protocol violation"));
            }
            other => panic!("expected protocol violation, got {other:?}"),
        }
    }

    fn fake_command(mode: &str) -> Command {
        let mut command = Command::new(fake_server_binary());
        command.env("AGENTMUX_FAKE_APP_SERVER_MODE", mode);
        command
    }

    fn fake_server_binary() -> &'static PathBuf {
        static BINARY: OnceLock<PathBuf> = OnceLock::new();
        BINARY.get_or_init(|| {
            let temp = tempfile::tempdir().expect("create fake server build dir");
            let mut binary = temp.path().join("fake-app-server");
            if cfg!(windows) {
                binary.set_extension("exe");
            }
            let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("fake_app_server.rs");
            let output = std::process::Command::new("rustc")
                .arg("--edition=2021")
                .arg(source)
                .arg("-o")
                .arg(&binary)
                .output()
                .expect("run rustc for fake App Server");
            assert!(
                output.status.success(),
                "fake App Server compilation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            std::mem::forget(temp);
            binary
        })
    }

    /// ReAgent P1, PR #3212 (6th review): a failure/timeout/cancellation
    /// while `write_value_tracked` is still awaiting the failure() check
    /// or the `outbound_tx.send(...)` step itself means the frame was
    /// never handed to `run_writer` at all -- genuinely safe to retry.
    #[tokio::test]
    async fn write_outcome_tracker_stays_safe_to_retry_when_the_write_never_starts() {
        let (client_io, _server_io) = duplex(4096);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let transport =
            AppServerTransport::new(client_reader, client_writer, AppServerLimits::default());
        fail_transport(&transport.shared, AppServerError::Closed);

        let tracker = WriteOutcomeTracker::new();
        let result = transport
            .write_value_tracked(json!({"x": 1}), &tracker)
            .await;
        assert!(result.is_err());
        assert!(tracker.safe_to_retry());
    }

    /// See write_outcome_tracker_stays_safe_to_retry_when_the_write_never_starts
    /// -- once the peer is fully gone, run_writer's actual write fails and
    /// reports that failure back through the confirmation ack, so the
    /// caller learns for certain the frame was never delivered and it
    /// becomes safe to retry again.
    #[tokio::test]
    async fn write_outcome_tracker_becomes_safe_to_retry_on_a_confirmed_write_failure() {
        let (client_io, server_io) = duplex(4096);
        drop(server_io);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let transport =
            AppServerTransport::new(client_reader, client_writer, AppServerLimits::default());

        let tracker = WriteOutcomeTracker::new();
        let result = transport
            .write_value_tracked(json!({"x": 1}), &tracker)
            .await;
        assert!(result.is_err(), "the write should fail once the peer is gone");
        assert!(
            tracker.safe_to_retry(),
            "a confirmed write failure means the frame was genuinely never delivered"
        );
    }

    /// ReAgent P1, PR #3212 (6th review): once the frame is handed to
    /// run_writer, it will be delivered independent of anything that
    /// happens to this call afterward. A caller whose future is
    /// cancelled while still waiting on the confirmation ack must not
    /// treat that the same as "never sent" -- a 1-byte duplex buffer with
    /// nothing reading the other side guarantees the write stays stuck
    /// there deterministically, rather than racing a timing window.
    #[tokio::test]
    async fn write_outcome_tracker_stays_unconfirmed_when_cancelled_after_queueing() {
        let (client_io, _server_io) = duplex(1);
        let (client_reader, client_writer) = tokio::io::split(client_io);
        let transport = Arc::new(AppServerTransport::new(
            client_reader,
            client_writer,
            AppServerLimits::default(),
        ));
        let tracker = Arc::new(WriteOutcomeTracker::new());

        let task_transport = transport.clone();
        let task_tracker = tracker.clone();
        let handle = tokio::spawn(async move {
            task_transport
                .write_value_tracked(json!({"x": 1}), &task_tracker)
                .await
        });

        for _ in 0..100 {
            if !tracker.safe_to_retry() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            !tracker.safe_to_retry(),
            "the frame should already be queued (unconfirmed) while the write is stuck"
        );

        handle.abort();
        let _ = handle.await;

        assert!(
            !tracker.safe_to_retry(),
            "a cancelled write whose outcome was never confirmed must stay unsafe to retry"
        );
    }
}
