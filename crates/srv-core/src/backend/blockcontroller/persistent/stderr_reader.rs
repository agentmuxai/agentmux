// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The persistent CLI's stderr reader task, started by `spawn_process`
//! (`spawn.rs`): logs every line and reacts to a stale `--resume`.

use super::*;

/// What the stderr reader task captures from `spawn_process`.
pub(super) struct StderrReaderCtx {
    pub(super) stderr_pipe: tokio::process::ChildStderr,
    pub(super) block_id_stderr: String,
    pub(super) inner_stderr: Arc<Mutex<PersistentInner>>,
    pub(super) mstore_stderr: Option<Arc<Store>>,
    pub(super) event_bus_stderr: Option<Arc<EventBus>>,
    pub(super) attempted_resume_sid: Option<String>,
    pub(super) my_generation_stderr: u64,
}

impl PersistentSubprocessController {
    /// Body of the stderr reader task; `spawn_process` keeps its JoinHandle
    /// for the process waiter.
    pub(super) async fn run_stderr_reader(ctx: StderrReaderCtx) {
        let StderrReaderCtx {
            stderr_pipe,
            block_id_stderr,
            inner_stderr,
            mstore_stderr,
            event_bus_stderr,
            attempted_resume_sid,
            my_generation_stderr,
        } = ctx;
        let mut reader = BufReader::new(stderr_pipe).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::warn!(
                block_id = %block_id_stderr,
                line = %line,
                "persistent stderr"
            );
            // Claude Code's own message when `--resume <sid>` targets a
            // conversation its current CLAUDE_CONFIG_DIR can't see — e.g.
            // after a relogin/reseed moves the agent onto a different
            // config dir than the one the session was recorded under. Left
            // uncleared, EVERY future respawn (one per message, since a
            // dead persistent process auto-restarts on next send) keeps
            // retrying the same unreachable --resume and immediately
            // exits again — a permanent "Agent encountered an error" with
            // no path to recovery. Clear it so the next respawn starts a
            // fresh conversation instead.
            if line.contains("No conversation found with session ID") {
                if let Some(ref bad_sid) = attempted_resume_sid {
                    // See PersistentInner::poison_resume — also guards
                    // against the stdout reader's own capture (`stdout_reader.rs`)
                    // re-adopting this same dead id if it wins the race.
                    inner_stderr.lock().unwrap().poison_resume(bad_sid, my_generation_stderr);
                    tracing::warn!(
                        block_id = %block_id_stderr,
                        session_id = %bad_sid,
                        "stale --resume session id unreachable under the current config dir — \
                         clearing so the next message starts a fresh conversation"
                    );
                    core::persist_session_id(&block_id_stderr, "", &mstore_stderr, &event_bus_stderr);
                    // Surface this to the user — previously silent
                    // (only the warn! above). See
                    // SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md
                    // §4.2: a resumed conversation silently starting
                    // fresh, with no indication anything happened, is
                    // exactly the failure mode this flag exists to close.
                    if let Some(ref store) = mstore_stderr {
                        crate::backend::blockcontroller::session_recovery::mark_resume_failed(
                            store,
                            &event_bus_stderr,
                            &block_id_stderr,
                        );
                    }
                }
            }
        }
    }
}
