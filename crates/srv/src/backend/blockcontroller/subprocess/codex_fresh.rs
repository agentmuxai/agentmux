// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Agents fresh` for a Codex pane (`docs/specs/SPEC_AGENT_SELF_COMPACTION_2026_10_23.md` §3.3).
//!
//! A Codex pane runs one `codex exec` per turn and resumes its thread by id,
//! so ending the thread is forgetting the id: the next turn starts a new one.
//! Three places hold the id, and all three must let go in one step:
//!
//! - the controller's `session_id` (cleared here, under `inner`);
//! - the block's `agent:sessionid` meta (cleared right after; a message built
//!   from the meta in between, or one already queued behind a turn, can still
//!   carry the old id, so the id is also *retired*: `hydrate_session_id_from_config`
//!   refuses it);
//! - the queue itself: a queued message makes the pane busy, so it refuses.
//!
//! The idle test and the clearing are one `inner` acquisition, so a message
//! either started its turn first (refused; the caller waits and tries again) or
//! finds no thread to resume. The old thread stays in `$CODEX_HOME` and in
//! AgentMux's record (`Agents search_history`).

use std::sync::atomic::Ordering;

use super::{SubprocessController, SUBPROCESS_OUTPUT_SUBJECT};

/// Why a fresh session can't start yet; the caller waits and tries again.
/// `server/app_api/agent_context.rs` retries on this text.
pub(crate) const TURN_RUNNING: &str = "a turn is running";

impl SubprocessController {
    /// End the pane's Codex thread so the next message starts a new one, and
    /// tell the pane (the `fresh` session-outcome frame: its divider, the meter
    /// reset and the memory fallback follow from it). Only between turns.
    /// Returns the old thread id.
    pub(crate) fn start_fresh_session(&self) -> Result<Option<String>, &'static str> {
        let old = {
            let mut inner = self.inner.lock().unwrap();
            if self.run_lock.load(Ordering::SeqCst) || !inner.pending_messages.is_empty() || self.health_monitor.is_active_turn() {
                return Err(TURN_RUNNING);
            }
            let old = inner.session_id.take();
            if let Some(sid) = &old {
                inner.retired_sessions.push(sid.clone());
            }
            old
        };
        super::super::core::persist_session_id(&self.block_id, "", &self.mstore, &self.event_bus);
        if let Some(broker) = &self.broker {
            let zone = crate::backend::transcript::append::resolve_global_output_zone(&self.mstore, &self.block_id);
            super::super::shell::handle_append_block_file(
                broker,
                &self.block_id,
                SUBPROCESS_OUTPUT_SUBJECT,
                super::super::persistent::outcome_line::fresh_by_choice_line().as_bytes(),
                self.filestore.as_ref(),
                zone.as_deref(),
            );
        }
        Ok(old)
    }
}

#[cfg(test)]
mod tests {
    use super::super::SubprocessSpawnConfig;
    use super::*;
    use std::sync::Arc;

    fn controller() -> SubprocessController {
        let c = SubprocessController::new("tab".into(), "block".into(), None, None, None, None, None, Arc::from("boot"));
        c.inner.lock().unwrap().session_id = Some("old-thread".into());
        c
    }

    #[test]
    fn an_idle_pane_forgets_its_thread_and_never_resumes_it() {
        let c = controller();
        assert_eq!(c.start_fresh_session(), Ok(Some("old-thread".to_string())));
        assert_eq!(c.session_id(), None);
        // A message built from the meta before it was cleared still names the old thread.
        c.hydrate_session_id_from_config(Some("old-thread"));
        assert_eq!(c.session_id(), None, "the retired thread is not resumed");
        c.hydrate_session_id_from_config(Some("other-thread"));
        assert_eq!(c.session_id().as_deref(), Some("other-thread"), "another id still hydrates (a picker reattach)");
        assert!(SubprocessController::record_captured_session_id_inner(&c.inner, "new-thread"));
        assert_eq!(c.session_id().as_deref(), Some("new-thread"), "the new thread's own id is captured");
    }

    #[test]
    fn a_turn_in_progress_refuses_and_keeps_the_thread() {
        let c = controller();
        assert!(c.try_lock_run());
        assert_eq!(c.start_fresh_session(), Err(TURN_RUNNING));
        assert_eq!(c.session_id().as_deref(), Some("old-thread"));
        c.unlock_run();
        assert!(c.start_fresh_session().is_ok());
    }

    #[test]
    fn a_queued_message_refuses_so_it_cannot_resume_the_old_thread_behind_the_stop() {
        let c = controller();
        let queued = SubprocessSpawnConfig { session_id: Some("old-thread".into()), message: "hi".into(), ..Default::default() };
        c.inner.lock().unwrap().pending_messages.push_back(queued);
        assert_eq!(c.start_fresh_session(), Err(TURN_RUNNING), "the run lock is free but a message is about to run");
        assert_eq!(c.session_id().as_deref(), Some("old-thread"));
        c.inner.lock().unwrap().pending_messages.clear();
        assert!(c.start_fresh_session().is_ok());
    }

    #[test]
    fn an_active_turn_refuses() {
        let c = controller();
        c.health_monitor.set_active_turn(true);
        assert_eq!(c.start_fresh_session(), Err(TURN_RUNNING));
        c.health_monitor.set_active_turn(false);
        assert!(c.start_fresh_session().is_ok());
    }

    #[test]
    fn a_pane_with_no_thread_yet_starts_fresh_with_nothing_to_forget() {
        let c = SubprocessController::new("tab".into(), "block".into(), None, None, None, None, None, Arc::from("boot"));
        assert_eq!(c.start_fresh_session(), Ok(None));
    }

    #[test]
    fn the_pane_is_told_it_is_a_fresh_session_with_a_handoff_on_its_way() {
        let frame: serde_json::Value = serde_json::from_str(super::super::super::persistent::outcome_line::fresh_by_choice_line().trim()).unwrap();
        assert_eq!(frame["subtype"], "agentmux_session_outcome");
        assert_eq!(frame["outcome"], "fresh");
        assert_eq!(frame["idle_start"], false);
        assert_eq!(frame["attempted_sid"], "");
    }
}
