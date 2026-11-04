// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `start` that open and wire the PTY: marking the pane running with its input
//! channel, opening the PTY, spawning the child, taking the reader and writer, and starting the
//! output reader and flusher.

use portable_pty::{native_pty_system, CommandBuilder, PtyPair};
use tokio::sync::mpsc;

use super::super::{BlockInputUnion, STATUS_DONE, STATUS_RUNNING};
use super::controller::ShellController;
use super::lifecycle::{run_pty_output_flusher, PtyReadChunk};
use super::pty::PTY_CHANNEL_CAPACITY;

/// The PTY's reader and writer.
pub(super) type PtyIo = (
    Box<dyn std::io::Read + Send>,
    Box<dyn std::io::Write + Send>,
);

impl ShellController {
    /// Marks the pane running, creates its input channel and publishes the status. Returns the
    /// receiving end for the input task.
    pub(super) fn mark_running(&self, conn_name: &str) -> mpsc::UnboundedReceiver<BlockInputUnion> {
        // Update status to running.
        {
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_RUNNING);
            inner.conn_name = conn_name.to_string();
        }

        // Create input channel. Unbounded: input must never be silently
        // dropped on burst (the paste-truncation bug). See SHELL_INPUT_CH_SIZE.
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        {
            let mut inner = self.inner.lock().unwrap();
            inner.input_tx = Some(input_tx);
            inner.input_seq_next = 0;
            inner.input_seq_buf.clear();
        }

        // Publish "running" only AFTER `input_tx` exists, so the event is a
        // truthful readiness signal: a frontend that resizes the PTY the instant
        // it sees "running" will not hit `send_input`'s "controller is not
        // running" guard (which fires while `input_tx` is None). The channel is
        // unbounded, so a resize enqueued before the input task spawns is
        // buffered, not lost.
        // See docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
        self.publish_status();
        input_rx
    }

    /// Opens the PTY at the size the pane asked for.
    pub(super) fn open_pty(&self, rt_opts: &Option<serde_json::Value>) -> Result<PtyPair, String> {
        // Real PTY path.
        //
        // Open the PTY at the size the frontend computed from the pane (passed
        // as `rtopts.termsize` on the resync), so the agent CLI and its tools
        // wrap correctly from the very first byte. The earlier code always
        // opened at a fixed 200 cols and relied on a post-spawn resize RPC to
        // correct it — but that RPC races controller startup and could fail
        // outright, leaving output wrapped at 200 all session. Seeding the size
        // here removes that race; `pty_size_from_rt_opts` falls back to the
        // historical 25x200 when no size was supplied (e.g. programmatic
        // spawns). See docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
        let pty_system = native_pty_system();
        let pty_size = self.spawn_pty_size(rt_opts);

        let pair = pty_system.openpty(pty_size).map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, "failed to open PTY");
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to open PTY: {e}")
        })?;
        tracing::info!(block_id = %self.block_id, rows = pty_size.rows, cols = pty_size.cols, "PTY opened");
        Ok(pair)
    }

    /// Spawns `cmd` on the PTY's slave side; a failure ends the start and frees the run lock.
    pub(super) fn spawn_child(
        &self,
        pair: &PtyPair,
        mut cmd: CommandBuilder,
        cmd_str: &str,
        is_agent: bool,
    ) -> Result<Box<dyn portable_pty::Child + Send + Sync>, String> {
        if is_agent {
            crate::backend::process_tracker::registry::place_pty_spawn(&self.block_id, &mut cmd);
        }
        let child = pair.slave.spawn_command(cmd).map_err(|e| {
            tracing::error!(block_id = %self.block_id, error = %e, cmd = %cmd_str, "spawn failed");
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to spawn command: {e}")
        })?;
        tracing::info!(block_id = %self.block_id, "process spawned successfully");
        Ok(child)
    }

    /// The PTY's reader and writer; a failure kills the child and ends the start.
    pub(super) fn take_pty_io(
        &self,
        pair: &PtyPair,
        child: &mut Box<dyn portable_pty::Child + Send + Sync>,
    ) -> Result<PtyIo, String> {
        // Get reader/writer from master
        let reader = pair.master.try_clone_reader().map_err(|e| {
            let _ = child.kill();
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to clone PTY reader: {e}")
        })?;

        let writer = pair.master.take_writer().map_err(|e| {
            let _ = child.kill();
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_DONE);
            inner.proc_exit_code = -1;
            inner.input_tx = None;
            self.unlock_run();
            format!("failed to take PTY writer: {e}")
        })?;
        Ok((reader, writer))
    }

    /// Starts the blocking PTY reader and the coalescing flusher it feeds. Returns the flusher's
    /// handle and a weak sender for the post-exit flush barrier.
    pub(super) fn start_output(
        &self,
        reader: Box<dyn std::io::Read + Send>,
        is_agent: bool,
    ) -> (tokio::task::JoinHandle<()>, mpsc::WeakSender<PtyReadChunk>) {
        // Spawn PTY read task (blocking I/O → spawn_blocking) + a companion
        // coalescing flusher (async task). Split in two, connected by an
        // mpsc channel, so the actual OS-level `read()` loop's blocking
        // semantics are untouched (this file supports both Unix and
        // Windows PTYs and that boundary is not something to risk getting
        // subtly wrong per-platform) while the expensive per-chunk work —
        // file write, WebSocket broadcast, OSC/translation — is batched.
        // See docs/reports/REPORT_RENDERER_CPU_UNBATCHED_PTY_OUTPUT_2026_09_11.md.
        let block_id_read = self.block_id.clone();
        let block_id_flush = self.block_id.clone();
        let broker_read = self.broker.clone();
        let broker_flush = self.broker.clone();
        let inner_read = self.inner.clone();
        let filestore_flush = self.filestore.clone();
        let is_agent_read = is_agent;
        let is_agent_flush = is_agent;
        // Bounded (reagentx P1 on PR #3206) — see PTY_CHANNEL_CAPACITY's own
        // doc comment for why: an unbounded channel here would let a
        // sustained fast producer grow queued memory without limit if the
        // flusher ever falls behind the read rate.
        let (pty_tx, pty_rx) = mpsc::channel::<PtyReadChunk>(PTY_CHANNEL_CAPACITY);
        // Kept for the post-exit flush barrier (see the wait/cleanup task).
        //
        // A WEAK sender, emphatically not a clone. ReAgent P1 (round 3) on PR
        // #3261: an mpsc channel closes only once EVERY `Sender` is dropped, so
        // a strong clone held by the wait task keeps the channel open for as
        // long as that task lives — which is precisely across the window where
        // the task waits for the flusher to end. `flusher_handle` could then
        // never resolve, on any platform, making the "prefer true EOF" fast
        // path dead code and silently applying the fallback's residual race to
        // every exit everywhere. An earlier revision of this very comment
        // claimed a clone "does NOT keep the channel open"; that was simply
        // wrong.
        //
        // `WeakSender` does not count towards keeping the channel alive, and
        // `upgrade()` returning `None` is itself the answer the barrier wants:
        // the channel is closed, so everything in it was drained.
        let barrier_tx: tokio::sync::mpsc::WeakSender<PtyReadChunk> = pty_tx.downgrade();

        super::pty_tasks::spawn_pty_reader(
            reader,
            pty_tx,
            inner_read,
            broker_read,
            is_agent_read,
            block_id_read,
        );

        // Coalescing flusher: batches whatever chunks the read loop above
        // has already queued (bounded by PTY_COALESCE_MAX_BYTES, and never
        // waiting for more to arrive) before doing the
        // actual file write + broadcast + translation — once per batch
        // instead of once per (up to 4KB) PTY read. A standalone fn (not
        // inlined into the spawn) so it can be driven directly in tests
        // with a synthetic channel, no real PTY required.
        //
        // Handle kept (reagentx P1 on PR #3206, third round): the wait
        // task below must await this before publishing STATUS_DONE — see
        // its own comment for why.
        let flusher_handle = tokio::spawn(run_pty_output_flusher(
            pty_rx,
            broker_flush,
            block_id_flush,
            filestore_flush,
            is_agent_flush,
        ));
        (flusher_handle, barrier_tx)
    }
}
