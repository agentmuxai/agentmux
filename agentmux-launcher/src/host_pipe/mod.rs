// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CPD-2 — launcher → host pipe wrapper.
//
// This module wraps the writer half of the launcher's named-pipe
// connection to the host process and exposes a stable interface for
// the rest of the launcher to push **Commands** (saga-issued, future
// CPD-3 wiring) and **Events** (existing reducer fanout) down to the
// host as `HostFrame` envelopes.
//
// CPD-2 is **infrastructure-only**: the saga coordinator's
// `apply_action` for `IssueCmd::Host` STAYS log-only. CPD-3 replaces
// the log with a `HostPipe::send_command` call.
//
// See `docs/specs/SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md` §3.5 +
// §3.9 for the design + spec acceptance criteria.
//
// ## Lifecycle / connection model
//
// The IPC server's per-connection fanout task hands the host's writer
// half to `HostPipe::set_writer` once the connecting client has
// registered as `ClientKind::Host`. On disconnect (read EOF or write
// failure), the same task calls `HostPipe::clear_writer`, which arms
// the disconnect timer (30s — see §3.9). Reconnect re-arms via
// `set_writer` again.
//
// While disconnected, `send_command` BUFFERS into a bounded
// `pending_buffer` (cap 64 frames). On reconnect, frames drain in
// FIFO order. On overflow OR after 30s of disconnection, the
// pending buffer is flushed and **`Event::SagaActionFailed` is
// emitted on the broadcast bus for every Command frame whose saga
// got dropped.** Event frames are silently dropped on overflow —
// they're already broadcast to other clients via the per-connection
// fanout, so the host missing one isn't a saga-correctness issue.
//
// Until CPD-1 (schema PR) lands, `Event::SagaActionFailed` does not
// exist on the agentmux-common Event enum. To avoid a hard dep on
// CPD-1's merge order, this module emits failures via the existing
// `Event::SagaFailed { saga_id, reason, version }` event with a
// `reason` prefix of `"host pipe ..."` so a downstream observer can
// filter. CPD-3 + CPD-1 will tighten the variant surface.
//
// ## HostFrame envelope
//
// Per spec §3.1, the launcher → host wire eventually carries a
// tagged union of Event vs Command. Until CPD-1 lands the envelope
// in agentmux-common, we declare `HostFrame` here locally. Both
// CPD-1 and CPD-2 can land independently; whichever lands first
// owns the envelope, the second PR migrates over.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agentmux_common::ipc::{Command, Event};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;

mod connection;
#[cfg(test)]
mod tests;

/// Maximum number of frames buffered while the host is disconnected.
/// On overflow, the oldest frame is dropped; if it was a Command,
/// `Event::SagaFailed` is emitted for the saga whose dispatch was
/// lost. See spec §3.9.
pub const PENDING_BUFFER_CAP: usize = 64;

/// Maximum disconnect duration before the pending buffer is flushed.
/// After this, every buffered Command's saga is failed with
/// `"host unreachable"`. See spec §3.9.
pub const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// CPD-1/CPD-2 envelope for the launcher → host pipe. Every wire line
/// the launcher writes to the host pipe is a `HostFrame` serialized as
/// newline-delimited JSON.
///
/// **Frame discriminator:** externally tagged via `kind` so the host
/// can dispatch `event` frames into its existing event-handling code
/// and `command` frames into the new (CPD-3+) saga-driven command
/// handlers without a separate channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostFrame {
    Event(Event),
    Command(Command),
}

/// Reasons `send_command` / `send_event` can fail synchronously.
///
/// `HostNotConnected` is reserved for cases where the caller wants to
/// **fail-fast** instead of buffering (CPD-3 will choose whichever
/// fits per command kind). The default `send_command` code path
/// buffers; see `try_send_command_no_buffer` for the fail-fast variant.
#[derive(Debug)]
pub enum HostPipeError {
    /// No host is currently connected and the caller asked to fail
    /// rather than buffer.
    #[allow(dead_code)] // reserved for fail-fast callers (CPD-3+)
    HostNotConnected,
    /// Underlying write to the pipe failed.
    WriteFailed(io::Error),
    /// Frame failed to serialize. Should be impossible for well-
    /// formed Command / Event variants but kept as a result variant
    /// for defense-in-depth.
    #[allow(dead_code)] // defense-in-depth; serde_json::to_vec on owned types should not fail
    Serialize(serde_json::Error),
    /// Pending buffer is full and we couldn't drop the oldest entry
    /// to make room (e.g. caller asked for no-overflow semantics).
    /// Default `send_command` does drop-oldest + emit-failed, so
    /// callers won't see this from the public API today.
    #[allow(dead_code)] // reserved for stricter callers
    BufferOverflow,
}

impl std::fmt::Display for HostPipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostPipeError::HostNotConnected => write!(f, "host not connected"),
            HostPipeError::WriteFailed(e) => write!(f, "write to host pipe failed: {}", e),
            HostPipeError::Serialize(e) => write!(f, "frame serialize failed: {}", e),
            HostPipeError::BufferOverflow => write!(f, "host pipe pending buffer overflow"),
        }
    }
}

impl std::error::Error for HostPipeError {}

/// Trait-object writer reference the HostPipe holds when connected.
///
/// `Arc<Mutex<Box<dyn AsyncWrite + Unpin + Send>>>` so the same
/// physical write half is reachable both by HostPipe (event fanout +
/// saga command dispatch) AND by the IPC server's per-connection
/// handler (connection-private error replies — Event::Error on parse
/// failures + register-first violations). The mutex serializes
/// writes so frames can't interleave on the wire.
///
/// On set, HostPipe stores a clone of the same Arc the per-connection
/// handler uses for its main loop. On clear, HostPipe drops its
/// clone; the handler still has its own Arc so error replies (if
/// any) keep working through the disconnect path.
pub type SharedWriter = Arc<Mutex<BoxedWriter>>;

/// Test-only convenience: own a Box directly. Tests construct one
/// via `make_shared_writer(Box::new(duplex_a))`.
pub type BoxedWriter = Box<dyn AsyncWrite + Unpin + Send>;

/// Wrap a boxed writer into a SharedWriter. Used by tests +
/// `ipc::server` to coerce a concrete WriteHalf into the trait
/// object the HostPipe stores.
///
/// Implementation note: we go through an intermediate
/// `Arc<Mutex<Box<dyn AsyncWrite + ...>>>` and then move the inner
/// Box out at write time. This avoids relying on
/// `Arc<Mutex<Concrete>>`-to-`Arc<Mutex<dyn>>` unsizing coercion,
/// which is unstable across MSRVs for `tokio::sync::Mutex`.
pub fn make_shared_writer(writer: BoxedWriter) -> SharedWriter {
    Arc::new(Mutex::new(writer))
}

/// One frame waiting to be written to the host. Tracks the saga_id
/// when known so we can emit a meaningful `SagaFailed` if the frame
/// is dropped on overflow / timeout.
#[derive(Debug)]
struct PendingFrame {
    frame: HostFrame,
    /// `Some(id)` for Command frames whose Command variant carries
    /// a saga_id. `None` for Event frames (the host is not the only
    /// subscriber for events — they also flow to the renderer / Tool
    /// clients via the broadcast bus, so a missed event-frame is not
    /// a saga-level failure) and for Command frames whose variant
    /// has no saga_id field yet (the schema additions land in CPD-1).
    saga_id: Option<u64>,
}

struct HostPipeInner {
    writer: Option<SharedWriter>,
    pending_buffer: VecDeque<PendingFrame>,
    /// Set when the writer transitions from Some -> None. Cleared on
    /// reconnect. Used to enforce the 30s disconnect timeout.
    host_disconnected_at: Option<Instant>,
}

impl HostPipeInner {
    fn new() -> Self {
        Self {
            writer: None,
            pending_buffer: VecDeque::with_capacity(PENDING_BUFFER_CAP),
            host_disconnected_at: None,
        }
    }
}

/// Public wrapper around the launcher → host pipe.
///
/// Cheap to clone (one `Arc`); shared by the IPC server's per-
/// connection task (set/clear writer) and the saga coordinator
/// (send_command, CPD-3) and the per-host fanout task (send_event,
/// refactored in this PR).
///
/// Manual `Debug` (rather than derive) so we don't have to require
/// `Debug` on the boxed writer trait object.
#[derive(Clone)]
pub struct HostPipe {
    inner: Arc<Mutex<HostPipeInner>>,
    /// Broadcast bus reference — used to emit `Event::SagaFailed`
    /// when a buffered Command is dropped (overflow or 30s timeout).
    events_tx: tokio::sync::broadcast::Sender<Event>,
    /// Reference to the launcher's reducer state for `bump_version`
    /// when emitting saga lifecycle events. Mirrors the saga
    /// coordinator's pattern so emitted events get a fresh,
    /// monotonic global version.
    state: Arc<Mutex<crate::state::State>>,
}

impl std::fmt::Debug for HostPipe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostPipe").finish_non_exhaustive()
    }
}

impl HostPipe {
    pub fn new(
        events_tx: tokio::sync::broadcast::Sender<Event>,
        state: Arc<Mutex<crate::state::State>>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HostPipeInner::new())),
            events_tx,
            state,
        }
    }

    /// Hand the host's writer half to the pipe. Called by the IPC
    /// server's per-connection task once the connecting client
    /// registers as `ClientKind::Host`. Drains any pending frames
    /// in FIFO order before returning.
    ///
    /// If a writer is already set (a prior host registered and this
    /// is a second), the prior writer is replaced. The drain logic
    /// still runs against the new writer.
    ///
    /// **Locking:** the inner mutex is held across the per-frame
    /// `write_all().await`. That's intentional — concurrent
    /// `send_frame` calls during drain would race FIFO ordering
    /// against the freshly-installed writer. Holding the lock keeps
    /// the drain-then-install transition atomic from the caller's
    /// perspective. Hold time is bounded by `pending_buffer` size
    /// (cap 64) × per-frame write latency (microseconds on a healthy
    /// pipe), well under the existing `state.lock()` discipline.
    pub async fn set_writer(&self, writer: SharedWriter) {
        let mut inner = self.inner.lock().await;
        inner.host_disconnected_at = None;
        // Drain pending frames against the new writer in FIFO order.
        while let Some(p) = inner.pending_buffer.pop_front() {
            let mut wlock = writer.lock().await;
            let res = write_frame_async(&mut **wlock, &p.frame).await;
            drop(wlock);
            if let Err(e) = res {
                crate::log(&format!(
                    "[host_pipe] drain failed mid-flight: {} — re-buffering rest",
                    e
                ));
                inner.pending_buffer.push_front(p);
                inner.host_disconnected_at = Some(Instant::now());
                return;
            }
        }
        inner.writer = Some(writer);
    }

    /// Mark the host as disconnected. Arms the 30s timer; subsequent
    /// `send_command` calls buffer up to `PENDING_BUFFER_CAP`.
    pub async fn clear_writer(&self) {
        let mut inner = self.inner.lock().await;
        if inner.writer.is_some() {
            inner.writer = None;
            inner.host_disconnected_at = Some(Instant::now());
            crate::log("[host_pipe] host disconnected — buffering subsequent frames (30s budget)");
        }
    }

    /// Push a Command down to the host. On disconnect, buffers up to
    /// `PENDING_BUFFER_CAP`; on overflow or 30s timeout, drops + emits
    /// `Event::SagaFailed` for the dropped Command's saga.
    ///
    /// CPD-2 wires this method but does NOT call it from the saga
    /// coordinator yet — that's CPD-3.
    #[allow(dead_code)] // CPD-3 wires this into the saga coordinator
    pub async fn send_command(&self, cmd: &Command) -> Result<(), HostPipeError> {
        let saga_id = saga_id_of(cmd);
        let frame = HostFrame::Command(cmd.clone());
        self.send_frame(frame, saga_id).await
    }

    /// Push an Event down to the host. Called by the per-connection
    /// fanout task in `ipc::server` for the host's connection (replaces
    /// the prior direct `send_event` call). Other client kinds keep
    /// the direct path because they're not subject to HostPipe's
    /// pending-buffer / reconnect semantics.
    pub async fn send_event(&self, event: &Event) -> Result<(), HostPipeError> {
        let frame = HostFrame::Event(event.clone());
        self.send_frame(frame, None).await
    }

    async fn send_frame(
        &self,
        frame: HostFrame,
        saga_id: Option<u64>,
    ) -> Result<(), HostPipeError> {
        // First, check the disconnect timer. If we've been disconnected
        // > 30s, drain the pending buffer + emit failures BEFORE we
        // queue a fresh frame on top.
        self.expire_pending_if_timed_out().await;

        // Take a clone of the writer Arc under the inner lock, then
        // release the inner lock before doing I/O. This avoids
        // serializing inner-state access (pending buffer, timer)
        // behind the wire write — multiple concurrent send_frame
        // calls then queue at the writer's mutex, not the inner's.
        let writer_clone = {
            let inner = self.inner.lock().await;
            inner.writer.clone()
        };
        if let Some(w) = writer_clone {
            let mut wlock = w.lock().await;
            match write_frame_async(&mut **wlock, &frame).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    drop(wlock);
                    crate::log(&format!(
                        "[host_pipe] direct write failed: {} — clearing writer + buffering",
                        e
                    ));
                    let mut inner = self.inner.lock().await;
                    inner.writer = None;
                    inner.host_disconnected_at = Some(Instant::now());
                    inner
                        .pending_buffer
                        .push_back(PendingFrame { frame, saga_id });
                    Err(HostPipeError::WriteFailed(e))
                }
            }
        } else {
            // Disconnected — buffer. Overflow drops oldest + emits
            // SagaFailed if it was a tagged Command.
            let mut inner = self.inner.lock().await;
            let to_drop = if inner.pending_buffer.len() >= PENDING_BUFFER_CAP {
                inner.pending_buffer.pop_front()
            } else {
                None
            };
            inner
                .pending_buffer
                .push_back(PendingFrame { frame, saga_id });
            drop(inner);
            if let Some(dropped) = to_drop {
                self.emit_drop_failure(dropped, "host pipe backpressure overflow")
                    .await;
            }
            Ok(())
        }
    }

    /// If the disconnect timer has elapsed, flush the pending buffer
    /// and emit `Event::SagaFailed` for every buffered Command.
    async fn expire_pending_if_timed_out(&self) {
        let now = Instant::now();
        let drained: Vec<PendingFrame> = {
            let mut inner = self.inner.lock().await;
            let Some(start) = inner.host_disconnected_at else {
                return;
            };
            if now.duration_since(start) < DISCONNECT_TIMEOUT {
                return;
            }
            // Reset the timer so we don't re-drain on the next call;
            // the next disconnect transition rearms it.
            inner.host_disconnected_at = None;
            inner.pending_buffer.drain(..).collect()
        };
        if !drained.is_empty() {
            crate::log(&format!(
                "[host_pipe] disconnect exceeded {:?}; dropping {} buffered frames",
                DISCONNECT_TIMEOUT,
                drained.len()
            ));
        }
        for f in drained {
            self.emit_drop_failure(f, "host unreachable").await;
        }
    }

    async fn emit_drop_failure(&self, dropped: PendingFrame, reason: &str) {
        let Some(saga_id) = dropped.saga_id else {
            // Event frames + Command frames without a saga_id are
            // silently dropped — they don't gate any saga's progress.
            return;
        };
        let v = {
            let mut state = self.state.lock().await;
            state.bump_version()
        };
        let evt = Event::SagaFailed {
            saga_id,
            reason: reason.to_string(),
            version: v,
        };
        let _ = self.events_tx.send(evt);
    }

    /// Test-only inspection of the current pending buffer length.
    #[cfg(test)]
    pub async fn pending_len(&self) -> usize {
        self.inner.lock().await.pending_buffer.len()
    }

    /// Test-only inspection of whether a writer is currently set.
    #[cfg(test)]
    pub async fn is_connected(&self) -> bool {
        self.inner.lock().await.writer.is_some()
    }

    /// Test-only: force the disconnect timer back by `delta` so the
    /// 30s timeout fires without sleeping in tests.
    #[cfg(test)]
    pub async fn rewind_disconnect_timer(&self, delta: Duration) {
        let mut inner = self.inner.lock().await;
        if let Some(t) = inner.host_disconnected_at.as_mut() {
            *t = t.checked_sub(delta).unwrap_or(*t);
        }
    }
}

/// Pull the saga_id off a Command if its variant carries one.
///
/// Today (pre-CPD-1) the host-bound Command variants don't have
/// `saga_id` fields — that's the schema-addition work in CPD-1. So
/// every variant returns `None`, which means CPD-2's drop semantics
/// don't actually fail a saga yet. CPD-1 + CPD-3 together will start
/// returning Some(id) for the relevant variants and the drop
/// machinery becomes saga-correctness-relevant. Keeping the
/// indirection here means CPD-3's diff against the saga coordinator
/// stays narrow.
fn saga_id_of(_cmd: &Command) -> Option<u64> {
    // CPD-1 will replace the body of this match with per-variant
    // returns of `Some(*saga_id)` for SpawnPoolWindow / ReapPanes /
    // DrainPoolIfLast etc. once those variants gain the field.
    None
}

/// Serialize a `HostFrame` as newline-delimited JSON and write it
/// to a writer. Used both for direct writes (connected) and for
/// pending-buffer drain (post-reconnect).
async fn write_frame_async<W: AsyncWrite + Unpin + ?Sized>(
    writer: &mut W,
    frame: &HostFrame,
) -> io::Result<()> {
    let mut buf = serde_json::to_vec(frame)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    buf.push(b'\n');
    writer.write_all(&buf).await?;
    writer.flush().await?;
    Ok(())
}
