// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The task that watches a shell's PTY child: it reaps the child, lets the output
//! flusher drain, hands the pane back (status, run lock, registrations) and, if the
//! pane was set to close on exit, closes it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::super::{BlockControllerRuntimeStatus, STATUS_DONE, STATUS_RUNNING};
use super::controller::{ShellController, ShellControllerInner};
use super::lifecycle::{flush_barrier, release_lease_if_current, PtyReadChunk};
use super::pty::{FLUSHER_BARRIER_TIMEOUT, FLUSHER_EOF_GRACE};
use crate::backend::eventbus::EventBus;
use crate::backend::mps;
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

/// Everything the wait task needs from `start`, moved in when it is spawned.
pub(super) struct ExitWatch {
    pub(super) child: Box<dyn portable_pty::Child + Send + Sync>,
    /// The askpass secret lives exactly as long as the shell.
    pub(super) askpass_guard: Option<crate::backend::remote::askpass::Revoke>,
    /// The SSH connection name and destination, for an SSH pane.
    pub(super) ssh: Option<(String, String)>,
    pub(super) inner: Arc<Mutex<ShellControllerInner>>,
    pub(super) close_on_exit: bool,
    pub(super) close_on_exit_delay_ms: u64,
    pub(super) block_id: String,
    pub(super) tab_id: String,
    pub(super) agent_id: Option<String>,
    pub(super) broker: Option<Arc<mps::Broker>>,
    pub(super) filestore: Option<Arc<FileStore>>,
    pub(super) mstore: Option<Arc<Store>>,
    pub(super) event_bus: Option<Arc<EventBus>>,
    pub(super) run_lock: Arc<AtomicBool>,
    pub(super) flusher_handle: tokio::task::JoinHandle<()>,
    pub(super) barrier_tx: tokio::sync::mpsc::WeakSender<PtyReadChunk>,
}

/// What `start` hands over to build an [`ExitWatch`]; the rest comes from the controller.
pub(super) struct ExitParts {
    pub(super) child: Box<dyn portable_pty::Child + Send + Sync>,
    /// The askpass secret lives exactly as long as the shell.
    pub(super) askpass_guard: Option<crate::backend::remote::askpass::Revoke>,
    pub(super) ssh: Option<(String, String)>,
    pub(super) agent_id: Option<String>,
    pub(super) close_on_exit: bool,
    pub(super) close_on_exit_delay_ms: u64,
    pub(super) flusher_handle: tokio::task::JoinHandle<()>,
    pub(super) barrier_tx: tokio::sync::mpsc::WeakSender<PtyReadChunk>,
}

impl ShellController {
    /// The exit watcher for a started shell: the pieces `start` made, plus the controller's own state.
    pub(super) fn exit_watch(&self, parts: ExitParts) -> ExitWatch {
        let ExitParts {
            child,
            askpass_guard,
            ssh,
            agent_id,
            close_on_exit,
            close_on_exit_delay_ms,
            flusher_handle,
            barrier_tx,
        } = parts;
        ExitWatch {
            child,
            askpass_guard,
            ssh,
            inner: Arc::clone(&self.inner),
            close_on_exit,
            close_on_exit_delay_ms,
            block_id: self.block_id.clone(),
            tab_id: self.tab_id.clone(),
            agent_id,
            broker: self.broker.clone(),
            filestore: self.filestore.clone(),
            mstore: self.mstore.clone(),
            event_bus: self.event_bus.clone(),
            run_lock: Arc::clone(&self.run_lock),
            flusher_handle,
            barrier_tx,
        }
    }
}

impl ExitWatch {
    pub(super) fn spawn(self) {
        tokio::spawn(self.run());
    }

    async fn run(self) {
        let ExitWatch {
            child,
            askpass_guard: askpass_guard_wait,
            ssh: ssh_wait,
            inner: inner_wait,
            close_on_exit,
            close_on_exit_delay_ms,
            block_id: block_id_wait,
            tab_id: tab_id_wait,
            agent_id: agent_id_wait,
            broker: broker_wait,
            filestore: filestore_note,
            mstore: mstore_wait,
            event_bus: event_bus_wait,
            run_lock,
            flusher_handle,
            barrier_tx,
        } = self;
        let (exit_code, raw_exit_code) = reap_child(child, block_id_wait.clone()).await;
        drop(askpass_guard_wait);
        // An SSH pane that lost its connection stays, saying so (ssh_pane).
        let lost_note = ssh_wait.as_ref().and_then(|(conn, dest)| {
            let stopped_by_srv = inner_wait.lock().unwrap().proc_status != STATUS_RUNNING;
            super::ssh_pane::ended(conn, dest, raw_exit_code, stopped_by_srv)
        });
        let close_on_exit = close_on_exit && lost_note.is_none();

        tracing::info!(block_id = %block_id_wait, exit_code = exit_code, "process exited");

        drain_flusher(close_on_exit, flusher_handle, barrier_tx, &block_id_wait).await;

        // After the drain above, so it comes after ssh's own last words.
        if let (Some(text), Some(broker)) = (lost_note, broker_wait.clone()) {
            super::ssh_pane::write_note(broker, filestore_note, block_id_wait.clone(), text).await;
        }

        // Kept for the close-on-exit step, after the teardown has taken the originals.
        let block_id_for_close = block_id_wait.clone();
        let tab_id_for_close = tab_id_wait.clone();
        // Identity of THIS controller instance, for the still-the-current-controller
        // check in `close_pane_later`.
        let inner_for_close = Arc::clone(&inner_wait);

        // `Teardown::run` does its work on a blocking thread: registry::remove and
        // the rest do synchronous file I/O, which doesn't belong on an async worker.
        Teardown {
            block_id: block_id_wait,
            agent_id: agent_id_wait,
            inner: inner_wait,
            mstore: mstore_wait,
            event_bus: event_bus_wait,
            broker: broker_wait,
            run_lock,
            exit_code,
        }
        .run()
        .await;

        if close_on_exit {
            close_pane_later(
                close_on_exit_delay_ms,
                block_id_for_close,
                tab_id_for_close,
                inner_for_close,
            );
        }
    }
}

/// Reaps the child on a blocking thread: it depends only on the direct child exiting, never
/// on PTY EOF. Gives the pane's exit code (0, 1 or -1) and the raw code for an SSH pane.
async fn reap_child(
    child: Box<dyn portable_pty::Child + Send + Sync>,
    block_id_wait: String,
) -> (i32, Option<u32>) {
    let block_id_reap = block_id_wait.clone();
    tokio::task::spawn_blocking(move || {
        let mut child = child;
        match child.wait() {
            // The pane keeps its 0 / 1 / -1; the raw code is for an
            // SSH pane, where 255 means ssh could not connect.
            Ok(status) => (
                if status.success() { 0 } else { 1 },
                Some(status.exit_code()),
            ),
            Err(e) => {
                tracing::warn!("wait error for block {}: {}", block_id_reap, e);
                (-1, None)
            }
        }
    })
    .await
    .expect("PTY child wait task panicked")
}

/// Waits, bounded, for the output flusher to drain what the child wrote last.
async fn drain_flusher(
    close_on_exit: bool,
    flusher_handle: tokio::task::JoinHandle<()>,
    barrier_tx: tokio::sync::mpsc::WeakSender<PtyReadChunk>,
    block_id_wait: &str,
) {
    // Bounded wait for the flusher to drain trailing output — see
    // `FLUSHER_DRAIN_TIMEOUT`'s own doc comment for why this must be
    // bounded. Deliberately does NOT `abort()` on timeout (reagentx
    // P1, same round, on the timeout+abort version of this fix):
    // the flusher (consumer) and the PTY read loop (producer,
    // `spawn_blocking` doing a raw, un-cancellable OS-level
    // `reader.read()`) are separate tasks, unlike persistent/stdout_reader.rs's
    // single combined reader task. Aborting only the flusher would
    // stop it draining `pty_tx` — but the read loop, still blocked
    // in real OS I/O, cannot be interrupted by tokio's cooperative
    // cancellation and keeps running regardless, so the leaked
    // thread is NOT reclaimed either way. Aborting would only add a
    // regression on top: every later `blocking_send` finds the
    // receiver gone and is silently discarded, permanently losing
    // any further output from a still-live descendant with no log
    // at the point of loss. Simply letting `flusher_handle` drop
    // here (no `abort()`) detaches it — the flusher and its read
    // loop keep running independently in the background, still
    // persisting and broadcasting whatever the descendant produces
    // next, and exit naturally once the PTY genuinely reaches EOF.
    //
    // Two signals, strongest first (ReAgent P1 on PR #3261).
    //
    // The flusher task ENDING is the strong one: it only ends when the
    // channel closes, which only happens once the read loop observed
    // true EOF and dropped its sender — so every byte the child ever
    // produced is both enqueued and committed. Wherever EOF actually
    // arrives (Linux, macOS) this resolves in milliseconds and the
    // guarantee is exactly what it was before the barrier existed.
    //
    // The barrier is the fallback, for where EOF never comes at all:
    // on Windows ConPTY's console host outlives the direct child, so
    // the read loop stays blocked in a read that never returns and the
    // task can never end. Waiting on it there always burned the full
    // ceiling (§12) — the second a human sees after typing `exit`.
    //
    // Residual, stated plainly: the barrier cannot order against a
    // chunk the read loop has read but not yet enqueued, so on the
    // fallback path a final chunk racing it can land after
    // STATUS_DONE. Pre-PR behaviour on that same path was also
    // best-effort (it timed out and detached), so this trades grace
    // for responsiveness rather than trading away a guarantee that was
    // ever actually held there. Where the guarantee WAS held — EOF
    // platforms — it still is.
    //
    // Total worst case stays FLUSHER_DRAIN_TIMEOUT: the two waits
    // split that budget rather than each taking it (ReAgent P2).
    if close_on_exit {
        drop(flusher_handle);
    } else {
        match tokio::time::timeout(FLUSHER_EOF_GRACE, flusher_handle).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::warn!(
                    block_id = %block_id_wait,
                    error = %e,
                    "PTY output flusher task panicked or was cancelled before completing; \
                     proceeding with pane teardown anyway — some trailing output may be missing"
                );
            }
            Err(_) => {
                // No EOF within the grace — assume it is never coming
                // (ConPTY) and settle for the bounded barrier.
                // `None` means every sender is gone — the channel
                // closed and the flusher drained it, which is the
                // guarantee the barrier exists to obtain.
                let barrier = match barrier_tx.upgrade() {
                    Some(tx) => flush_barrier(&tx, FLUSHER_BARRIER_TIMEOUT).await,
                    None => Ok(()),
                };
                if barrier.is_err() {
                    tracing::warn!(
                        block_id = %block_id_wait,
                        timeout = ?FLUSHER_BARRIER_TIMEOUT,
                        "PTY output flush barrier went unanswered after process exit (a                                  wedged flusher, or a background descendant still holding the PTY                                  open?) — proceeding with pane teardown; the flusher and its read                                  loop stay detached and running, so later output is still not lost,                                  but some trailing output may land after STATUS_DONE"
                    );
                }
            }
        }
    }
}

/// What the blocking part of a pane's teardown needs.
struct Teardown {
    block_id: String,
    agent_id: Option<String>,
    inner: Arc<Mutex<ShellControllerInner>>,
    mstore: Option<Arc<Store>>,
    event_bus: Option<Arc<EventBus>>,
    broker: Option<Arc<mps::Broker>>,
    run_lock: Arc<AtomicBool>,
    exit_code: i32,
}

impl Teardown {
    /// Unregisters the pane, releases its lease, publishes DONE and frees the run lock.
    async fn run(self) {
        let Teardown {
            block_id: block_id_wait,
            agent_id: agent_id_wait,
            inner: inner_wait,
            mstore: mstore_wait,
            event_bus: event_bus_wait,
            broker: broker_wait,
            run_lock,
            exit_code,
        } = self;
        tokio::task::spawn_blocking(move || {
            unregister_pane(&block_id_wait, agent_id_wait.as_deref());

            // Release any PtyShell agent lease this block still holds.
            //
            // The lease (`blockcontroller::agent_lock`) locks a human out
            // of their own keyboard while an agent drives the pane's
            // shell, and it is deliberately time-bounded so it always
            // lapses on its own. But "the shell this lease belongs to has
            // exited" is a stronger signal than the clock: there is
            // nothing left to collide with, and holding the lock for the
            // remainder of its window would drop keystrokes aimed at
            // whatever comes next in this pane. Matters most for the
            // exact case that motivated close-on-exit
            // (SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md §10): a human
            // typing `exit` in a shell an agent was using must not have
            // their following keystrokes swallowed by a lease for a PTY
            // that is already gone.
            //
            // Guarded on this task still belonging to the REGISTERED
            // controller, the same `Arc::ptr_eq` identity check the
            // close-on-exit check in `close_pane_later` already uses, and for the same
            // reason (Codex P1 / ReAgent P1 on PR #3249): a force-restart
            // (`resync_controller` → `stop_for_replace`) kills this child
            // and registers a REPLACEMENT controller for the same
            // block_id. This wait task belongs to the OLD one. Without
            // the guard it would fire afterwards and clear a lease the
            // agent had legitimately taken on the NEW shell, letting
            // human input race the agent's writes on a pane that never
            // stopped being driven. Leases are keyed by block_id, so a
            // stale task cannot tell its own lease from its successor's —
            // controller identity is what distinguishes them.
            //
            // Check-then-act, like its precedent: a replacement
            // registered in the window between this check and the remove
            // would still lose its lease. Same residual race the
            // close-on-exit guard accepts, and self-correcting here — the
            // agent's next write re-takes the lease, since
            // `lock_shell_for_agent` runs before every one.
            release_lease_if_current(&block_id_wait, &inner_wait, &mstore_wait, &event_bus_wait);

            // Update inner state
            {
                let mut inner = inner_wait.lock().unwrap();
                inner.proc_exit_code = exit_code;
                ShellController::set_status(&mut inner, STATUS_DONE);
                inner.input_tx = None;
            }

            publish_done(&inner_wait, &broker_wait, &block_id_wait);

            // Release run lock
            run_lock.store(false, Ordering::SeqCst);
        })
        .await
        .expect("PTY wait/cleanup task panicked");
    }
}

/// Closes the pane after `close_on_exit_delay_ms`, if this controller is still the registered one.
fn close_pane_later(
    close_on_exit_delay_ms: u64,
    block_id_for_close: String,
    tab_id_for_close: String,
    inner_for_close: Arc<Mutex<ShellControllerInner>>,
) {
    // Close-on-exit (SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md
    // §10): fire-and-forget, deliberately AFTER STATUS_DONE is
    // published and `run_lock` is released by `Teardown::run` — closing the
    // pane doesn't need to block or gate either of those.
    //
    // Reached promptly after the child is reaped: for a CLOSING
    // pane the flusher-drain wait (`drain_flusher`) is skipped outright (§13),
    // so none of `FLUSHER_DRAIN_TIMEOUT` is paid here, and
    // `close_on_exit_delay_ms` defaults to 0. A pane that is NOT
    // closing still takes the full ordered drain wait — now
    // bounded at 1s rather than the original 10s (§12), because
    // that ceiling turned out to be reached on essentially every
    // ordinary exit on Windows (ConPTY's console host outlives
    // the direct child, so the read loop never sees EOF) and it
    // gates `STATUS_DONE` publication, i.e. every
    // STATUS_DONE-driven affordance — a pane's "done" icon,
    // `PtyShellStatus`'s `running: false` — not just this one.
    //
    // NOT gated on the flusher-drain outcome logged by `drain_flusher`.
    // An earlier version of this code skipped closing whenever the
    // flusher timed out, reasoning that a still-live background
    // descendant might have more output coming. Live-tested on
    // Windows (this PR) and found to backfire completely: the
    // 10s `FLUSHER_DRAIN_TIMEOUT` was hit on essentially EVERY
    // plain `cmd.exe` exit, not as a rare edge case — ConPTY's own
    // console-host process routinely outlives the direct child by
    // design, so gating on this made close-on-exit silently never
    // fire on the one platform this was tested on. Simply closing
    // regardless matches what actually happens next either way:
    // the flusher and its read loop are ALREADY left running
    // detached in the background (see `drain_flusher`), so any
    // genuinely-still-arriving trailing output keeps being
    // persisted to the block's file exactly as it would if the
    // pane stayed open — closing the pane just stops anyone from
    // looking at it, the same as closing any other terminal app's
    // window while a background job still has an open handle.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(close_on_exit_delay_ms)).await;
        // Only close if THIS controller is still the one
        // registered for the block (Codex P1 on PR #3230).
        //
        // A process exit is not by itself evidence that the
        // pane should close — the child also dies when the
        // controller is deliberately replaced. `resync_controller`'s
        // force-restart path (the terminal's own refresh
        // action, `forcerestart: true`) calls
        // `stop_for_replace`, which kills this child, then
        // registers a REPLACEMENT controller for the same
        // block. This wait task belongs to the OLD controller
        // and still carries `close_on_exit == true`, so
        // without this check reaping that killed child would
        // delete the pane — and with it the freshly
        // registered replacement — instead of completing the
        // restart. The delay widens the same window for
        // an explicitly-configured `cmd:closeonexitdelay`:
        // restarting an exited shell during it must cancel the
        // close, which re-checking AFTER the sleep is what
        // makes true.
        //
        // Registry identity is the discriminator, and it
        // covers the whole replace sequence: during it the old
        // entry is removed (`remove_controller_entry_only`)
        // and a new one registered, so this lookup finds
        // either nothing or a DIFFERENT controller — both
        // correctly skip. A natural exit leaves this
        // controller registered and untouched, so it matches
        // and the close proceeds.
        let still_current = super::super::get_controller(&block_id_for_close)
            .and_then(|ctrl| {
                ctrl.as_any()
                    .downcast_ref::<ShellController>()
                    .map(|sc| Arc::ptr_eq(&sc.inner, &inner_for_close))
            })
            .unwrap_or(false);
        if !still_current {
            tracing::info!(
                block_id = %block_id_for_close,
                "close-on-exit: controller was replaced or removed since this \
                 process exited (restart in flight?) — skipping pane close"
            );
            return;
        }
        super::super::close_on_exit(tab_id_for_close, block_id_for_close);
    });
}

/// Takes the pane out of the per-pane metrics, the jekt registry and the cloud subscriber.
fn unregister_pane(block_id_wait: &str, agent_id: Option<&str>) {
    // Unregister PID from per-pane metrics
    super::super::pidregistry::unregister(block_id_wait);

    // Deregister from jekt — removes the agent_id → block_id mapping so
    // subsequent jekt attempts fall back to MessageBus rather than a dead PTY.
    crate::backend::reactive::get_global_handler().unregister_block(block_id_wait);
    // Every file registered for this block, under whatever name it is filed by: the one removal by name
    // below misses an entry a turn wrote for the pane's current name.
    {
        let data_dir = crate::backend::base::get_mux_data_dir();
        crate::backend::reactive::registry::remove_for_block(&data_dir, block_id_wait, None);
        crate::backend::reactive::registry::remove_shared_for_block_from_env(block_id_wait, None);
    }

    // Also remove from cross-instance file registry and cloud subscriber, unless another live pane
    // still answers to the name. An agent's own tool shell carries the agent's id, so its exit
    // would otherwise unsubscribe the agent's running pane: its review and CI notices then expired
    // undelivered for three hours (Agent2@narko, 2026-11-10) until its next input turn.
    if let Some(agent_id) = agent_id {
        let still_held = crate::backend::reactive::get_global_handler().has_live_name(agent_id);
        if drop_name_state_on_exit(still_held) {
            let data_dir = crate::backend::base::get_mux_data_dir();
            crate::backend::reactive::registry::remove(&data_dir, agent_id);
            crate::backend::reactive::registry::remove_shared_from_env(agent_id);
            if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                sub.remove_agent(agent_id);
            }
        } else {
            tracing::info!(block_id = %block_id_wait, agent = %agent_id, "shell pane exit: another live pane holds the agent's name — keeping its registry entry and cloud subscription");
        }
    }
}

/// Whether an exiting pane takes down the name-keyed state (file registry entry, cloud
/// subscription) for its agent: only when no other live pane still answers to the name.
fn drop_name_state_on_exit(name_still_held: bool) -> bool {
    !name_still_held
}

#[cfg(test)]
mod name_state_tests {
    use super::drop_name_state_on_exit;

    #[test]
    fn an_exiting_shell_leaves_the_name_alone_while_a_pane_holds_it() {
        assert!(!drop_name_state_on_exit(true), "the agent's pane still runs: keep its subscription");
        assert!(drop_name_state_on_exit(false), "nobody holds the name: take its state down");
    }
}

/// Publishes the pane's DONE status to the broker, if there is one.
fn publish_done(
    inner_wait: &Arc<Mutex<ShellControllerInner>>,
    broker_wait: &Option<Arc<mps::Broker>>,
    block_id_wait: &str,
) {
    // Publish done status
    if let Some(ref broker) = broker_wait {
        let status = {
            let inner = inner_wait.lock().unwrap();
            BlockControllerRuntimeStatus {
                durable: false,
                blockid: block_id_wait.to_string(),
                version: inner.status_version,
                shellprocstatus: inner.proc_status.clone(),
                shellprocconnname: inner.conn_name.clone(),
                shellprocexitcode: inner.proc_exit_code,
                // Kept on the exit status too, so the drawer's
                // info line can still say which PID exited.
                shellprocpid: inner.child_pid,
                shellprocname: inner.shell_name.clone(),
                spawn_ts_ms: inner.spawn_ts_ms,
                is_agent_pane: inner.is_agent_pane,
                turn_active: false,
            }
        };
        super::super::publish_controller_status(broker, &status);
    }
}
