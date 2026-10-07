// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether each agent is working, waiting on the user, idle, stopped or
//! failed, and since when: the `agent_status` the LAN fleet feed,
//! `GET /agentmux/reactive/agent-names` and (through the feed's snapshot) the
//! cloud presence record carry (agentmux-mobile's
//! SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §3, §13.1).
//!
//! [`decide`] is the rule, pure; [`current_state`] reads its inputs from the
//! block's controller and the block's meta; a [`SinceTracker`] remembers when
//! each block's state began. Every caller goes through
//! [`agent_status_of_block`] or a [`SinceTracker`] of its own, so the feed
//! and the names route agree on `since_ms`.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Serialize;

use crate::backend::blockcontroller;
use crate::backend::storage::store::Store;

/// Block meta the agent pane sets while a question waits for the user.
pub const META_AWAITING_USER: &str = "term:awaiting_user";

/// An agent's state as the wire reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Working,
    /// A turn is running and a question waits for the user.
    Waiting,
    Idle,
    /// The process exited cleanly.
    Stopped,
    /// The process exited with an error.
    Error,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Idle => "idle",
            Self::Stopped => "stopped",
            Self::Error => "error",
        }
    }
}

/// A state and when it began (unix ms, this machine's clock).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AgentStatus {
    pub state: AgentState,
    pub since_ms: u64,
}

/// What [`decide`] looks at, as read from the controller and the block.
#[derive(Debug, Clone, Copy, Default)]
pub struct StateInputs<'a> {
    pub controller_type: &'a str,
    /// `BlockControllerRuntimeStatus::turn_active`; an absent field is false.
    pub turn_active: bool,
    /// `shellprocstatus`: `init`, `running` or `done`.
    pub process_status: &'a str,
    pub exit_code: i32,
    pub awaiting_user: bool,
}

/// The state, or `None` when nothing tracks this agent's turns (a terminal
/// pane: `shell` / `cmd`, or any controller that isn't an agent's) or its
/// process state is unknown. Saying nothing beats a wrong `idle`.
///
/// A turn wins over the process state, as in the process broker's
/// `lifecycle_from`. The `subprocess` controller runs one process per turn,
/// so its process exiting cleanly between turns is `idle`, not `stopped`.
pub fn decide(inputs: &StateInputs) -> Option<AgentState> {
    let per_turn_process = match inputs.controller_type {
        blockcontroller::BLOCK_CONTROLLER_SUBPROCESS => true,
        blockcontroller::BLOCK_CONTROLLER_PERSISTENT
        | blockcontroller::BLOCK_CONTROLLER_ACP
        | blockcontroller::BLOCK_CONTROLLER_APP_SERVER => false,
        _ => return None,
    };
    if inputs.turn_active {
        return Some(if inputs.awaiting_user { AgentState::Waiting } else { AgentState::Working });
    }
    match inputs.process_status {
        blockcontroller::STATUS_RUNNING | blockcontroller::STATUS_INIT => Some(AgentState::Idle),
        blockcontroller::STATUS_DONE if inputs.exit_code != 0 => Some(AgentState::Error),
        blockcontroller::STATUS_DONE if per_turn_process => Some(AgentState::Idle),
        blockcontroller::STATUS_DONE => Some(AgentState::Stopped),
        _ => None,
    }
}

/// The block's state now: its controller's runtime status and the block's
/// `term:awaiting_user`. `None` without a controller, or when the block
/// can't be read.
pub fn current_state(mstore: &Store, block_id: &str) -> Option<AgentState> {
    let controller = blockcontroller::get_controller(block_id)?;
    let status = controller.get_runtime_status();
    let block = mstore.get::<crate::backend::obj::Block>(block_id).ok()??;
    decide(&StateInputs {
        controller_type: controller.controller_type(),
        turn_active: status.turn_active,
        process_status: &status.shellprocstatus,
        exit_code: status.shellprocexitcode,
        awaiting_user: crate::backend::obj::meta_get_bool(&block.meta, META_AWAITING_USER, false),
    })
}

/// When each block's current state began. A state seen for the first time
/// (srv start, a new agent) begins when it is seen; a state that changes
/// begins again; `None` forgets the block, so a stopped and restarted agent
/// does not inherit an old start.
#[derive(Default)]
pub struct SinceTracker {
    by_block: parking_lot::Mutex<HashMap<String, AgentStatus>>,
}

impl SinceTracker {
    /// Record `state` for `block_id` as seen at `now_ms`; returns it with the
    /// time it began.
    pub fn record(&self, block_id: &str, state: Option<AgentState>, now_ms: u64) -> Option<AgentStatus> {
        let mut by_block = self.by_block.lock();
        let Some(state) = state else {
            by_block.remove(block_id);
            return None;
        };
        match by_block.get(block_id) {
            Some(known) if known.state == state => Some(*known),
            _ => {
                let status = AgentStatus { state, since_ms: now_ms };
                by_block.insert(block_id.to_string(), status);
                Some(status)
            }
        }
    }

    /// Drop every block `keep` rejects (no longer listed).
    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
        self.by_block.lock().retain(|block_id, _| keep(block_id));
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.by_block.lock().len()
    }
}

static TRACKER: LazyLock<SinceTracker> = LazyLock::new(SinceTracker::default);

/// The tracker the live fleet source and the names route share.
pub fn global_tracker() -> &'static SinceTracker {
    &TRACKER
}

/// The agent on `block_id`: its state now and since when.
pub fn agent_status_of_block(mstore: &Store, block_id: &str) -> Option<AgentStatus> {
    TRACKER.record(block_id, current_state(mstore, block_id), agentmux_common::time::now_ms_u64())
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::any::Any;
    use std::sync::Arc;

    use crate::backend::blockcontroller::{BlockControllerRuntimeStatus, BlockInputUnion, Controller};
    use crate::backend::obj::MetaMapType;

    /// A registered controller that reports a fixed runtime status.
    pub struct StubController {
        pub block_id: String,
        pub controller_type: String,
        pub status: BlockControllerRuntimeStatus,
    }

    impl Controller for StubController {
        fn start(&self, _: MetaMapType, _: Option<serde_json::Value>, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn stop(&self, _: bool, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
            self.status.clone()
        }
        fn send_input(&self, _: BlockInputUnion, _: Option<u64>) -> Result<(), String> {
            Ok(())
        }
        fn controller_type(&self) -> &str {
            &self.controller_type
        }
        fn block_id(&self) -> &str {
            &self.block_id
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    /// Register a stub on `block_id`; remove it with
    /// `blockcontroller::delete_controller`.
    pub fn register_stub(block_id: &str, controller_type: &str, process_status: &str, turn_active: bool) {
        crate::backend::blockcontroller::register_controller(
            block_id,
            Arc::new(StubController {
                block_id: block_id.to_string(),
                controller_type: controller_type.to_string(),
                status: BlockControllerRuntimeStatus {
                    blockid: block_id.to_string(),
                    shellprocstatus: process_status.to_string(),
                    turn_active,
                    ..Default::default()
                },
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::blockcontroller::{
        BLOCK_CONTROLLER_ACP, BLOCK_CONTROLLER_APP_SERVER, BLOCK_CONTROLLER_CMD, BLOCK_CONTROLLER_PERSISTENT,
        BLOCK_CONTROLLER_SHELL, BLOCK_CONTROLLER_SUBPROCESS, BLOCK_CONTROLLER_TSUNAMI, STATUS_DONE, STATUS_INIT,
        STATUS_RUNNING,
    };

    fn inputs<'a>(controller_type: &'a str, turn_active: bool, process_status: &'a str) -> StateInputs<'a> {
        StateInputs { controller_type, turn_active, process_status, exit_code: 0, awaiting_user: false }
    }

    #[test]
    fn one_case_per_state() {
        let p = BLOCK_CONTROLLER_PERSISTENT;
        assert_eq!(decide(&inputs(p, true, STATUS_RUNNING)), Some(AgentState::Working));
        assert_eq!(
            decide(&StateInputs { awaiting_user: true, ..inputs(p, true, STATUS_RUNNING) }),
            Some(AgentState::Waiting)
        );
        assert_eq!(decide(&inputs(p, false, STATUS_RUNNING)), Some(AgentState::Idle));
        assert_eq!(decide(&inputs(p, false, STATUS_INIT)), Some(AgentState::Idle));
        assert_eq!(decide(&inputs(p, false, STATUS_DONE)), Some(AgentState::Stopped));
        assert_eq!(
            decide(&StateInputs { exit_code: 1, ..inputs(p, false, STATUS_DONE) }),
            Some(AgentState::Error)
        );
    }

    #[test]
    fn a_question_outside_a_turn_is_not_waiting() {
        let left_over = StateInputs { awaiting_user: true, ..inputs(BLOCK_CONTROLLER_PERSISTENT, false, STATUS_RUNNING) };
        assert_eq!(decide(&left_over), Some(AgentState::Idle));
    }

    #[test]
    fn absent_turn_active_means_no_turn() {
        // A turn that ended is an absent field: it deserializes to false.
        let status: blockcontroller::BlockControllerRuntimeStatus =
            serde_json::from_value(serde_json::json!({"blockid": "b", "shellprocstatus": "running"})).unwrap();
        assert!(!status.turn_active);
        assert_eq!(
            decide(&inputs(BLOCK_CONTROLLER_PERSISTENT, status.turn_active, &status.shellprocstatus)),
            Some(AgentState::Idle)
        );
    }

    #[test]
    fn terminal_panes_and_unknown_controllers_report_nothing() {
        for t in [BLOCK_CONTROLLER_SHELL, BLOCK_CONTROLLER_CMD, BLOCK_CONTROLLER_TSUNAMI, "", "future"] {
            assert_eq!(decide(&inputs(t, false, STATUS_RUNNING)), None, "{t}");
            assert_eq!(decide(&inputs(t, true, STATUS_RUNNING)), None, "{t} mid-turn");
        }
    }

    #[test]
    fn every_agent_controller_reports_a_state() {
        for t in [BLOCK_CONTROLLER_PERSISTENT, BLOCK_CONTROLLER_ACP, BLOCK_CONTROLLER_APP_SERVER, BLOCK_CONTROLLER_SUBPROCESS] {
            assert_eq!(decide(&inputs(t, true, STATUS_RUNNING)), Some(AgentState::Working), "{t}");
        }
    }

    #[test]
    fn an_unknown_process_status_reports_nothing() {
        assert_eq!(decide(&inputs(BLOCK_CONTROLLER_PERSISTENT, false, "")), None);
        assert_eq!(decide(&inputs(BLOCK_CONTROLLER_PERSISTENT, false, "weird")), None);
    }

    #[test]
    fn a_per_turn_process_exiting_cleanly_is_idle() {
        let s = BLOCK_CONTROLLER_SUBPROCESS;
        assert_eq!(decide(&inputs(s, false, STATUS_DONE)), Some(AgentState::Idle));
        assert_eq!(decide(&StateInputs { exit_code: 2, ..inputs(s, false, STATUS_DONE) }), Some(AgentState::Error));
    }

    #[test]
    fn states_serialize_lowercase() {
        for s in [AgentState::Working, AgentState::Waiting, AgentState::Idle, AgentState::Stopped, AgentState::Error] {
            assert_eq!(serde_json::to_value(s).unwrap(), serde_json::json!(s.as_str()));
        }
        assert_eq!(
            serde_json::to_value(AgentStatus { state: AgentState::Working, since_ms: 7 }).unwrap(),
            serde_json::json!({"state": "working", "since_ms": 7})
        );
    }

    #[test]
    fn since_is_kept_while_the_state_holds_and_reset_on_a_change() {
        let t = SinceTracker::default();
        let working = Some(AgentState::Working);
        assert_eq!(t.record("b", working, 100).unwrap().since_ms, 100, "first seen");
        assert_eq!(t.record("b", working, 200).unwrap().since_ms, 100, "unchanged");
        assert_eq!(
            t.record("b", Some(AgentState::Idle), 300),
            Some(AgentStatus { state: AgentState::Idle, since_ms: 300 })
        );
        assert_eq!(t.record("b", working, 400).unwrap().since_ms, 400, "back to working starts again");
    }

    #[test]
    fn no_state_forgets_the_block() {
        let t = SinceTracker::default();
        t.record("b", Some(AgentState::Idle), 100);
        assert_eq!(t.record("b", None, 200), None);
        assert_eq!(t.len(), 0);
        assert_eq!(t.record("b", Some(AgentState::Idle), 300).unwrap().since_ms, 300, "not the old start");
    }

    #[test]
    fn retain_drops_unlisted_blocks() {
        let t = SinceTracker::default();
        t.record("a", Some(AgentState::Idle), 1);
        t.record("b", Some(AgentState::Idle), 1);
        t.retain(|b| b == "a");
        assert_eq!(t.len(), 1);
        assert_eq!(t.record("a", Some(AgentState::Idle), 5).unwrap().since_ms, 1);
    }

    fn store() -> Store {
        Store::open_in_memory().expect("in-memory store")
    }

    fn insert_block(store: &Store, meta: &[(&str, serde_json::Value)]) -> String {
        let mut block = crate::backend::obj::Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
        for (k, v) in meta {
            block.meta.insert(k.to_string(), v.clone());
        }
        store.insert(&mut block).expect("insert block");
        block.oid
    }

    #[test]
    fn current_state_reads_the_controller_and_the_block() {
        let store = store();
        let asking = insert_block(&store, &[(META_AWAITING_USER, serde_json::json!(true))]);
        test_support::register_stub(&asking, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, true);
        assert_eq!(current_state(&store, &asking), Some(AgentState::Waiting));

        let quiet = insert_block(&store, &[]);
        test_support::register_stub(&quiet, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
        assert_eq!(current_state(&store, &quiet), Some(AgentState::Idle));

        let terminal = insert_block(&store, &[]);
        test_support::register_stub(&terminal, BLOCK_CONTROLLER_SHELL, STATUS_RUNNING, false);
        assert_eq!(current_state(&store, &terminal), None);

        let no_controller = insert_block(&store, &[]);
        assert_eq!(current_state(&store, &no_controller), None);

        let unreadable = format!("missing-{}", uuid::Uuid::new_v4());
        test_support::register_stub(&unreadable, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, true);
        assert_eq!(current_state(&store, &unreadable), None, "the block can't be read");

        for b in [&asking, &quiet, &terminal, &unreadable] {
            blockcontroller::delete_controller(b);
        }
    }
}
