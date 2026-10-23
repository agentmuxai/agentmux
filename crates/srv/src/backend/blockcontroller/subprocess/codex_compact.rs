// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `/compact` for a Codex pane.
//!
//! A Codex pane runs one `codex exec` per turn, and `exec` can't compact a
//! thread. `codex app-server` can (`thread/compact/start`), and it reads the
//! same threads from `$CODEX_HOME`. So a `/compact` sent to a Codex pane (the
//! composer's Compact button sends exactly that) starts no `exec` turn: srv
//! runs a short-lived app-server with the turn's own command, environment and
//! working directory, resumes the thread, compacts it, and stops it.
//!
//! The pane sees what it sees for Claude Code: `compaction_started` when it
//! begins, then a `compact_boundary` frame in the stream (the context-compacted
//! card, which also ends "Compacting…"), or Claude's compaction-failed notice.
//!
//! Verified against codex-cli 0.154.0 on 2026-10-22: a 12,595-token thread
//! that `codex exec` made came back at 4,706 in about 2 s, and
//! `codex exec resume` carried on from it, remembering the earlier turn.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::backend::blockcontroller::app_server::{AppServerIncoming, AppServerLimits, AppServerProcess};
use crate::backend::blockcontroller::app_server_protocol::CodexAppServerSession;
use crate::backend::blockcontroller::{core, STATUS_DONE, STATUS_RUNNING};

use super::{SubprocessController, SubprocessSpawnConfig, SUBPROCESS_OUTPUT_SUBJECT};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(20);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long compacting may take, from the request: one model call over the thread.
const COMPACT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Whether this turn is a Codex `/compact`.
pub(super) fn is_compact_request(config: &SubprocessSpawnConfig) -> bool {
    config.resume_strategy == "codex-exec" && config.message.trim() == "/compact"
}

/// The context before and after compacting, in tokens.
#[derive(Debug, PartialEq)]
struct Compacted {
    pre_tokens: u64,
    post_tokens: u64,
}

impl SubprocessController {
    /// Compact the pane's Codex thread instead of running `/compact` as a turn.
    /// Called from `spawn_turn` holding the run lock, which it keeps until the
    /// compaction ends, so a message sent meanwhile is queued and sent after.
    /// Like a turn it holds the session's cross-process lease, and Stop
    /// (`stop_subprocess`, through `kill_tx`) cancels it.
    pub(super) fn compact_codex_thread(
        &self,
        config: SubprocessSpawnConfig,
        thread_id: Option<String>,
    ) -> Result<(), String> {
        let Some(thread_id) = thread_id.filter(|t| !t.is_empty()) else {
            self.unlock_run();
            return Err("Nothing to compact yet: this Codex agent hasn't run a turn.".to_string());
        };
        let lease = match self.claim_compact_lease(&config, &thread_id) {
            Ok(lease) => lease,
            Err(e) => {
                self.unlock_run();
                return Err(e);
            }
        };
        self.emit_message_accepted(&config);
        let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<bool>();
        {
            let mut inner = self.inner.lock().unwrap();
            Self::set_status(&mut inner, STATUS_RUNNING);
            inner.kill_tx = Some(kill_tx);
        }
        self.health_monitor.set_active_turn(true);
        self.health_monitor.note_spawned();
        self.publish_status();
        if let Some(broker) = &self.broker {
            let started = agentmux_common::client_events::CompactionStarted {
                trigger: agentmux_common::client_events::CompactionTrigger::Manual,
                session_id: thread_id.clone(),
                started_at: chrono::Utc::now().to_rfc3339(),
            };
            // Live only, like the PreCompact hook's (`persist: 0`, #2378).
            broker.publish_event::<crate::backend::mps_events::CompactionStarted>(
                &started,
                vec![format!("block:{}", self.block_id)],
                0,
            );
        }
        let done = Arc::new(AtomicBool::new(false));
        self.renew_compact_lease(lease.clone(), Arc::clone(&done));
        let weak = self.self_ref.lock().unwrap().clone().unwrap_or_default();
        let block_id = self.block_id.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            // Stop sends on `kill_tx`. Dropping `run` then drops its App Server
            // process, which `kill_on_drop` stops.
            let outcome = tokio::select! {
                outcome = run(&block_id, &config, &thread_id) => Some(outcome),
                Ok(_) = &mut kill_rx => None,
            };
            done.store(true, Ordering::SeqCst);
            if let Some(ctrl) = weak.upgrade() {
                ctrl.finish_codex_compact(outcome, started.elapsed(), lease);
            }
        });
        Ok(())
    }

    /// The session lease a turn claims (`host_spawn::spawn_turn`), so a
    /// compaction never changes a thread another AgentMux process is running a
    /// turn on. `None` when leasing is off for this pane or its store failed.
    fn claim_compact_lease(&self, config: &SubprocessSpawnConfig, thread_id: &str) -> Result<Option<crate::registry::Lease>, String> {
        if config.instance_id.is_empty() {
            return Ok(None);
        }
        let Some(store) = &self.lease_store else { return Ok(None) };
        match store.claim(&config.instance_id, &self.boot_id, &self.block_id, Some(thread_id)) {
            Ok(lease) => Ok(Some(lease)),
            Err(crate::registry::LeaseError::HeldByOther { owner_boot_id, age_ms, .. }) => Err(format!(
                "session '{}' is already owned by another AgentMux process (boot {owner_boot_id}, renewed {age_ms}ms ago): \
                 refusing to compact it from two processes at once",
                config.instance_id
            )),
            Err(crate::registry::LeaseError::Io(e)) => {
                tracing::warn!(block_id = %self.block_id, error = %e, "lease claim failed (io); compacting without a lease");
                Ok(None)
            }
        }
    }

    /// Renew the lease while the compaction runs, as a turn's renewal task does.
    fn renew_compact_lease(&self, lease: Option<crate::registry::Lease>, done: Arc<AtomicBool>) {
        let (Some(store), Some(lease)) = (self.lease_store.clone(), lease) else { return };
        let block_id = self.block_id.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(crate::registry::RENEW_INTERVAL_MS));
            loop {
                interval.tick().await;
                if done.load(Ordering::SeqCst) {
                    break;
                }
                let (s, l) = (Arc::clone(&store), lease.clone());
                let renewed = tokio::task::spawn_blocking(move || s.renew(&l).map_err(|e| e.to_string()))
                    .await
                    .unwrap_or_else(|e| Err(format!("renew task failed: {e}")));
                if let Err(e) = renewed {
                    tracing::warn!(block_id = %block_id, error = %e, "lease renew failed during a codex compact");
                    break;
                }
            }
        });
    }

    /// Tell the pane how it went, release the lease and end the "turn". After
    /// a finished compaction, send what was queued; after Stop (`None`), leave
    /// the status Stop set and send nothing: the user stopped the agent.
    fn finish_codex_compact(&self, outcome: Option<Result<Compacted, String>>, took: Duration, lease: Option<crate::registry::Lease>) {
        let now = chrono::Utc::now().to_rfc3339();
        let uuid = uuid::Uuid::new_v4().to_string();
        let frame = match &outcome {
            Some(Ok(c)) => {
                tracing::info!(block_id = %self.block_id, pre = c.pre_tokens, post = c.post_tokens, "codex thread compacted");
                boundary_frame(c, took, &uuid, &now)
            }
            Some(Err(error)) => {
                tracing::warn!(block_id = %self.block_id, error = %error, "codex compact failed");
                failed_frame(error, &uuid, &now)
            }
            None => {
                tracing::info!(block_id = %self.block_id, "codex compact stopped");
                failed_frame("stopped before it finished", &uuid, &now)
            }
        };
        if let Some(broker) = &self.broker {
            let global_zone = crate::backend::transcript::append::resolve_global_output_zone(&self.mstore, &self.block_id);
            crate::backend::blockcontroller::shell::handle_append_block_file(
                broker,
                &self.block_id,
                SUBPROCESS_OUTPUT_SUBJECT,
                format!("{frame}\n").as_bytes(),
                self.filestore.as_ref(),
                global_zone.as_deref(),
            );
        }
        if let (Some(store), Some(lease)) = (&self.lease_store, &lease) {
            if let Err(e) = crate::backend::blocking::off_async_worker(|| store.release(lease)) {
                tracing::warn!(block_id = %self.block_id, error = %e, "lease release failed after a codex compact (self-heals via TTL expiry)");
            }
        }
        let Some(outcome) = outcome else {
            self.health_monitor.set_active_turn(false);
            self.publish_status();
            self.unlock_run();
            return;
        };
        let exit_code = if outcome.is_ok() { 0 } else { 1 };
        {
            let mut inner = self.inner.lock().unwrap();
            inner.proc_exit_code = exit_code;
            inner.kill_tx = None;
            Self::set_status(&mut inner, STATUS_DONE);
        }
        self.health_monitor.set_exited(exit_code);
        self.health_monitor.set_active_turn(false);
        self.publish_status();
        self.unlock_run();
        let next = self.inner.lock().unwrap().pending_messages.pop_front();
        if let Some(config) = next {
            if let Err(e) = self.spawn_turn(config) {
                tracing::warn!(block_id = %self.block_id, error = %e, "failed to spawn the turn queued behind a compact");
            }
        }
    }
}

/// Start an app-server like the pane's own turns, compact, and stop it.
async fn run(block_id: &str, config: &SubprocessSpawnConfig, thread_id: &str) -> Result<Compacted, String> {
    let mut command = crate::server::cli_handlers::make_cli_cmd(&config.cli_command);
    command.args(["app-server", "--listen", "stdio://"]);
    core::apply_working_dir(&mut command, block_id, &config.working_dir, &config.env_vars);
    #[cfg(windows)]
    {
        use agentmux_common::win32::NoWindow;
        command.no_window();
    }
    crate::backend::process_tracker::registry::place_spawn(block_id, &mut command, true);
    let process = AppServerProcess::spawn(command, AppServerLimits::default())
        .map_err(|e| format!("couldn't start the Codex App Server: {e}"))?;
    if let Some(pid) = process.pid() {
        crate::backend::process_tracker::registry::track_spawned_agent(block_id, pid);
    }
    let outcome = compact(&process, thread_id, &config.working_dir).await;
    let _ = process.shutdown(Duration::from_secs(2)).await;
    outcome
}

async fn compact(process: &AppServerProcess, thread_id: &str, cwd: &str) -> Result<Compacted, String> {
    process
        .transport
        .initialize(INITIALIZE_TIMEOUT)
        .await
        .map_err(|e| format!("the Codex App Server didn't start: {e}"))?;
    let session = CodexAppServerSession::new(process.transport.clone());
    session
        .resume_thread(thread_id.to_string(), None, (!cwd.is_empty()).then(|| cwd.to_string()), REQUEST_TIMEOUT)
        .await
        .map_err(|e| format!("couldn't open the thread: {e}"))?;
    session.compact_thread(REQUEST_TIMEOUT).await.map_err(|e| format!("couldn't start compacting: {e}"))?;
    let deadline = tokio::time::Instant::now() + COMPACT_TIMEOUT;
    let mut progress = Progress::default();
    loop {
        let incoming = tokio::time::timeout_at(deadline, process.transport.next_incoming())
            .await
            .map_err(|_| "compacting took longer than 10 minutes".to_string())?
            .ok_or_else(|| "the Codex App Server stopped before compacting finished".to_string())?;
        if let AppServerIncoming::Notification { method, params } = incoming {
            if let Some(done) = progress.note(&method, &params) {
                return done;
            }
        }
    }
}

/// What the app-server's notifications say about one compaction. Their order
/// (codex-cli 0.154.0): the resume's `thread/tokenUsage/updated` (the context
/// before), `item/started` of a `contextCompaction`, a token update (the
/// context after), `item/completed`, then `turn/completed`.
#[derive(Default)]
struct Progress {
    pre: Option<u64>,
    post: Option<u64>,
    started: bool,
    compacted: bool,
}

impl Progress {
    fn note(&mut self, method: &str, params: &Value) -> Option<Result<Compacted, String>> {
        let item_is_compaction = || params.pointer("/item/type").and_then(Value::as_str) == Some("contextCompaction");
        match method {
            "thread/tokenUsage/updated" => {
                let tokens = params.pointer("/tokenUsage/last/totalTokens").and_then(Value::as_u64);
                if self.started {
                    self.post = tokens.or(self.post);
                } else {
                    self.pre = tokens.or(self.pre);
                }
            }
            "item/started" if item_is_compaction() => self.started = true,
            "item/completed" if item_is_compaction() => self.compacted = true,
            "turn/completed" => {
                let status = params.pointer("/turn/status").and_then(Value::as_str).unwrap_or("");
                if status != "completed" || !self.compacted {
                    let why = params
                        .pointer("/turn/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| match status {
                            "" | "completed" => "compacting ended without compacting".to_string(),
                            other => format!("compacting was {other}"),
                        });
                    return Some(Err(why));
                }
                return Some(Ok(Compacted { pre_tokens: self.pre.unwrap_or(0), post_tokens: self.post.unwrap_or(0) }));
            }
            _ => {}
        }
        None
    }
}

/// Claude Code's `compact_boundary` frame, which the pane reads for any
/// provider (`frontend/app/view/agent/frame-intercepts.ts`).
fn boundary_frame(c: &Compacted, took: Duration, uuid: &str, now: &str) -> Value {
    json!({
        "type": "system",
        "subtype": "compact_boundary",
        "uuid": uuid,
        "timestamp": now,
        "compact_metadata": {
            "trigger": "manual",
            "pre_tokens": c.pre_tokens,
            "post_tokens": c.post_tokens,
            "duration_ms": took.as_millis() as u64,
        },
    })
}

/// Claude Code's compaction-failed status frame, naming Codex
/// (`frontend/app/view/agent/cli-notice.ts`).
fn failed_frame(error: &str, uuid: &str, now: &str) -> Value {
    json!({
        "type": "system",
        "subtype": "status",
        "compact_result": "failed",
        "compact_error": error,
        "provider": "codex",
        "uuid": uuid,
        "timestamp": now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(strategy: &str, message: &str) -> SubprocessSpawnConfig {
        SubprocessSpawnConfig { resume_strategy: strategy.into(), message: message.into(), ..Default::default() }
    }

    #[test]
    fn only_a_codex_slash_compact_is_caught() {
        assert!(is_compact_request(&config("codex-exec", "/compact")));
        assert!(is_compact_request(&config("codex-exec", "  /compact\n")));
        assert!(!is_compact_request(&config("codex-exec", "/compact the logs please")));
        assert!(!is_compact_request(&config("flag", "/compact")), "Claude's /compact goes to its CLI");
    }

    /// The order codex-cli 0.154.0 sent, recorded 2026-10-22.
    #[test]
    fn reads_the_context_before_and_after_from_the_notifications() {
        let mut p = Progress::default();
        let usage = |n: u64| json!({ "tokenUsage": { "last": { "totalTokens": n } } });
        let item = |t: &str| json!({ "item": { "type": t } });
        assert!(p.note("thread/tokenUsage/updated", &usage(12_595)).is_none());
        assert!(p.note("item/started", &item("contextCompaction")).is_none());
        assert!(p.note("thread/tokenUsage/updated", &usage(4_706)).is_none());
        assert!(p.note("item/completed", &item("contextCompaction")).is_none());
        let done = p.note("turn/completed", &json!({ "turn": { "status": "completed" } }));
        assert_eq!(done, Some(Ok(Compacted { pre_tokens: 12_595, post_tokens: 4_706 })));
    }

    #[test]
    fn a_turn_that_ends_without_compacting_is_a_failure() {
        let mut p = Progress::default();
        let failed = p.note("turn/completed", &json!({ "turn": { "status": "failed", "error": { "message": "usage limit reached" } } }));
        assert_eq!(failed, Some(Err("usage limit reached".to_string())));
        let mut p = Progress::default();
        let interrupted = p.note("turn/completed", &json!({ "turn": { "status": "interrupted" } }));
        assert_eq!(interrupted, Some(Err("compacting was interrupted".to_string())));
    }

    /// The frames are the ones the pane already parses for Claude Code.
    #[test]
    fn the_frames_are_claude_codes_shapes() {
        let b = boundary_frame(&Compacted { pre_tokens: 10, post_tokens: 4 }, Duration::from_millis(2_157), "u1", "t");
        assert_eq!(b["subtype"], "compact_boundary");
        assert_eq!(b["compact_metadata"], json!({ "trigger": "manual", "pre_tokens": 10, "post_tokens": 4, "duration_ms": 2_157 }));
        let f = failed_frame("boom", "u2", "t");
        assert_eq!((f["subtype"].as_str(), f["compact_result"].as_str(), f["provider"].as_str()), (Some("status"), Some("failed"), Some("codex")));
    }
}
