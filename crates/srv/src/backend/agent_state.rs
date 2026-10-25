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
//!
//! `limited` and `signed_out` are an agent that is alive but can't work: its
//! last run ended on the plan's usage limit or a refused sign-in
//! (`docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md` §3). Read them through
//! [`unavailable_of_block`], so no caller re-derives the rule.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::agents::failure::FailureClass;
use crate::backend::blockcontroller;
use crate::backend::obj::Block;
use crate::backend::storage::store::Store;

/// Block meta the agent pane sets while a question waits for the user.
pub const META_AWAITING_USER: &str = "term:awaiting_user";

/// The account a pane's run launched on, written at spawn
/// (`build_persistent_spawn_env`), so a failure that run records counts
/// against that account even after the agent is linked to another
/// ([`run_accounts_of_block`]).
pub const RUN_ACCOUNT_META_KEY: &str = "agent:run_account";

/// An agent's state as the wire reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
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
    /// Its last run ended on the plan's usage limit, which hasn't reset.
    Limited,
    /// Its last run ended on refused credentials, or no account to run on.
    #[serde(rename = "signed_out")]
    SignedOut,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Idle => "idle",
            Self::Stopped => "stopped",
            Self::Error => "error",
            Self::Limited => "limited",
            Self::SignedOut => "signed_out",
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
    /// The class and reset (unix ms) of the block's `agent:last_failure`.
    pub failure: Option<(FailureClass, Option<u64>)>,
    /// This machine's clock, unix ms, for the reset.
    pub now_ms: u64,
}

/// The state, or `None` when nothing tracks this agent's turns (a terminal
/// pane: `shell` / `cmd`, or any controller that isn't an agent's) or its
/// process state is unknown. Saying nothing beats a wrong `idle`.
///
/// A turn wins over everything, as in the process broker's `lifecycle_from`:
/// a turn that runs is evidence the agent can work. Then a usage limit that
/// hasn't reset (or whose reset isn't known) is `limited`, and a refused
/// sign-in `signed_out`, whatever the process state: anything started on that
/// account fails the same way. Other failure classes are transient and don't
/// change the state. The `subprocess` controller runs one process per turn, so
/// its process exiting cleanly between turns is `idle`, not `stopped`.
pub fn decide(inputs: &StateInputs) -> Option<AgentState> {
    let per_turn_process = match inputs.controller_type {
        blockcontroller::BLOCK_CONTROLLER_SUBPROCESS => true,
        blockcontroller::BLOCK_CONTROLLER_PERSISTENT
        | blockcontroller::BLOCK_CONTROLLER_ACP
        | blockcontroller::BLOCK_CONTROLLER_MSP
        | blockcontroller::BLOCK_CONTROLLER_APP_SERVER => false,
        _ => return None,
    };
    if inputs.turn_active {
        return Some(if inputs.awaiting_user { AgentState::Waiting } else { AgentState::Working });
    }
    match inputs.failure {
        Some((FailureClass::UsageLimit, resets)) if resets.is_none_or(|r| r > inputs.now_ms) => {
            return Some(AgentState::Limited);
        }
        Some((FailureClass::Auth, _)) => return Some(AgentState::SignedOut),
        _ => {}
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
/// `term:awaiting_user` and `agent:last_failure`. `None` without a controller,
/// or when the block can't be read.
pub fn current_state(mstore: &Store, block_id: &str) -> Option<AgentState> {
    current_state_at(mstore, block_id, agentmux_common::time::now_ms_u64())
}

/// [`current_state`] at `now_ms`.
pub fn current_state_at(mstore: &Store, block_id: &str, now_ms: u64) -> Option<AgentState> {
    let controller = blockcontroller::get_controller(block_id)?;
    let status = controller.get_runtime_status();
    let block = mstore.get::<Block>(block_id).ok()??;
    decide(&StateInputs {
        controller_type: controller.controller_type(),
        turn_active: status.turn_active,
        process_status: &status.shellprocstatus,
        exit_code: status.shellprocexitcode,
        awaiting_user: crate::backend::obj::meta_get_bool(&block.meta, META_AWAITING_USER, false),
        failure: failure_of_block(&block).map(|f| (f.code, f.resets_at_ms)),
        now_ms,
    })
}

/// The parts of a block's `agent:last_failure` the states read. Only these
/// fields: the stored failure leaves out empty ones that reading the whole
/// `AgentFailure` back would require.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFailure {
    pub code: FailureClass,
    pub title: String,
    pub resets_at_ms: Option<u64>,
}

/// The block's stored failure, or `None` when it has none (or one this build
/// can't read).
pub fn failure_of_block(block: &Block) -> Option<StoredFailure> {
    let f = block.meta.get(blockcontroller::core::META_LAST_FAILURE)?;
    Some(StoredFailure {
        code: serde_json::from_value(f.get("code")?.clone()).ok()?,
        title: f.get("title").and_then(|t| t.as_str()).unwrap_or_default().to_string(),
        resets_at_ms: f.get("resetsAtMs").and_then(|r| r.as_u64()),
    })
}

/// An agent that is alive but can't work, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unavailable {
    /// `limited` or `signed_out`.
    pub state: AgentState,
    pub code: FailureClass,
    pub title: String,
    /// When its usage limit lifts (unix ms), when the provider said.
    pub resets_at_ms: Option<u64>,
    /// Since when it has been in this state (unix ms).
    pub since_ms: u64,
    /// The account its run launched on. Never put in an answer another host
    /// may receive (MuxAPI, the fleet feed).
    #[serde(skip)]
    pub account_id: Option<String>,
}

impl Unavailable {
    /// "Out of quota until Sun 7:00 PM", "Out of quota" or "Needs sign-in",
    /// in this machine's time zone.
    pub fn reason(&self) -> String {
        match (self.state, self.resets_at_ms.and_then(|ms| chrono::DateTime::from_timestamp_millis(ms as i64))) {
            (AgentState::SignedOut, _) => "Needs sign-in".to_string(),
            (_, Some(t)) => format!("Out of quota until {}", t.with_timezone(&chrono::Local).format("%a %-I:%M %p")),
            (_, None) => "Out of quota".to_string(),
        }
    }
}

/// `Some` when the agent on `block_id` is `limited` or `signed_out` at
/// `now_ms`. With `identity`, a pane from before `agent:run_account` gets its
/// agent's current account link.
pub fn unavailable_of_block(mstore: &Store, identity: Option<&Store>, block_id: &str, now_ms: u64) -> Option<Unavailable> {
    let state = current_state_at(mstore, block_id, now_ms)?;
    if !matches!(state, AgentState::Limited | AgentState::SignedOut) {
        return None;
    }
    let block = mstore.get::<Block>(block_id).ok()??;
    let failure = failure_of_block(&block)?;
    let since_ms = TRACKER.record(block_id, Some(state), now_ms).map_or(now_ms, |s| s.since_ms);
    let account_id = match identity {
        Some(identity) => run_accounts_of_block(identity, &block).into_iter().next(),
        None => block.meta.get(RUN_ACCOUNT_META_KEY).and_then(|a| a.as_str()).map(str::to_string),
    };
    Some(Unavailable { state, code: failure.code, title: failure.title, resets_at_ms: failure.resets_at_ms, since_ms, account_id })
}

/// The block's state now and, while it is `limited` or `signed_out`, why.
pub fn state_and_unavailable(mstore: &Store, block_id: &str) -> (Option<AgentState>, Option<Unavailable>) {
    let state = agent_status_of_block(mstore, block_id).map(|s| s.state);
    (state, unavailable_of_block(mstore, None, block_id, agentmux_common::time::now_ms_u64()))
}

/// The accounts a pane's run counts against: `agent:run_account`, the account
/// it launched on; a pane from before that key falls back to its agent's
/// current links. One rule for the spawn gate's `account_blocks`,
/// [`unavailable_of_block`] and the account episodes.
pub fn run_accounts_of_block(identity: &Store, block: &Block) -> Vec<String> {
    match block.meta.get(RUN_ACCOUNT_META_KEY).and_then(|a| a.as_str()) {
        Some(account) => vec![account.to_string()],
        None => {
            let agent = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
            identity.agent_identity_list_for_agent(&agent).unwrap_or_default().into_iter().map(|l| l.account_id).collect()
        }
    }
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
        /// What started the turn in flight, for a test of something gated on it.
        pub provenance: Option<crate::backend::blockcontroller::health::TurnProvenance>,
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
        fn turn_provenance(&self) -> Option<crate::backend::blockcontroller::health::TurnProvenance> {
            self.provenance.clone()
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
                provenance: None,
            }),
        );
    }

    /// [`register_stub`] on a running persistent agent whose turn in flight
    /// has `provenance`.
    pub fn register_turn(block_id: &str, provenance: crate::backend::blockcontroller::health::TurnProvenance) {
        use crate::backend::blockcontroller::{BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING};
        crate::backend::blockcontroller::register_controller(
            block_id,
            Arc::new(StubController {
                block_id: block_id.to_string(),
                controller_type: BLOCK_CONTROLLER_PERSISTENT.to_string(),
                status: BlockControllerRuntimeStatus {
                    blockid: block_id.to_string(),
                    shellprocstatus: STATUS_RUNNING.to_string(),
                    turn_active: true,
                    ..Default::default()
                },
                provenance: Some(provenance),
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::blockcontroller::{
        BLOCK_CONTROLLER_ACP, BLOCK_CONTROLLER_APP_SERVER, BLOCK_CONTROLLER_CMD, BLOCK_CONTROLLER_MSP, BLOCK_CONTROLLER_PERSISTENT,
        BLOCK_CONTROLLER_SHELL, BLOCK_CONTROLLER_SUBPROCESS, STATUS_DONE, STATUS_INIT,
        STATUS_RUNNING,
    };

    fn inputs<'a>(controller_type: &'a str, turn_active: bool, process_status: &'a str) -> StateInputs<'a> {
        StateInputs { controller_type, turn_active, process_status, exit_code: 0, awaiting_user: false, failure: None, now_ms: 1_000 }
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
        for t in [BLOCK_CONTROLLER_SHELL, BLOCK_CONTROLLER_CMD, "", "future"] {
            assert_eq!(decide(&inputs(t, false, STATUS_RUNNING)), None, "{t}");
            assert_eq!(decide(&inputs(t, true, STATUS_RUNNING)), None, "{t} mid-turn");
        }
    }

    #[test]
    fn every_agent_controller_reports_a_state() {
        for t in [BLOCK_CONTROLLER_PERSISTENT, BLOCK_CONTROLLER_ACP, BLOCK_CONTROLLER_MSP, BLOCK_CONTROLLER_APP_SERVER, BLOCK_CONTROLLER_SUBPROCESS] {
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
        for s in [
            AgentState::Working,
            AgentState::Waiting,
            AgentState::Idle,
            AgentState::Stopped,
            AgentState::Error,
            AgentState::Limited,
            AgentState::SignedOut,
        ] {
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

    /// The full table: turn active × process state × failure class × reset
    /// (spec §3.2, §10).
    #[test]
    fn a_limit_or_a_refused_sign_in_wins_over_the_process_but_not_over_a_turn() {
        let now = 1_000;
        let resets = [None, Some(now - 1), Some(now), Some(now + 1)];
        let classes = [FailureClass::UsageLimit, FailureClass::Auth, FailureClass::RateLimited, FailureClass::Network];
        for controller in [BLOCK_CONTROLLER_PERSISTENT, BLOCK_CONTROLLER_SUBPROCESS] {
            for process in [STATUS_RUNNING, STATUS_INIT, STATUS_DONE] {
                for exit_code in [0, 1] {
                    let base = StateInputs { exit_code, now_ms: now, ..inputs(controller, false, process) };
                    let plain = decide(&base);
                    for class in classes {
                        for reset in resets {
                            let failed = StateInputs { failure: Some((class, reset)), ..base };
                            let mid_turn = StateInputs { turn_active: true, ..failed };
                            assert_eq!(decide(&mid_turn), Some(AgentState::Working), "a running turn wins: {class:?} {reset:?}");
                            let expected = match (class, reset) {
                                (FailureClass::UsageLimit, None) => Some(AgentState::Limited),
                                (FailureClass::UsageLimit, Some(r)) if r > now => Some(AgentState::Limited),
                                (FailureClass::Auth, _) => Some(AgentState::SignedOut),
                                _ => plain,
                            };
                            assert_eq!(decide(&failed), expected, "{controller} {process} exit {exit_code} {class:?} {reset:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_terminal_pane_with_a_failure_still_reports_nothing() {
        let failed = StateInputs { failure: Some((FailureClass::UsageLimit, None)), ..inputs(BLOCK_CONTROLLER_SHELL, false, STATUS_RUNNING) };
        assert_eq!(decide(&failed), None);
    }

    #[test]
    fn signed_out_round_trips_with_its_underscore() {
        assert_eq!(serde_json::to_value(AgentState::SignedOut).unwrap(), serde_json::json!("signed_out"));
        assert_eq!(serde_json::from_value::<AgentState>(serde_json::json!("signed_out")).unwrap(), AgentState::SignedOut);
        assert_eq!(serde_json::from_value::<AgentState>(serde_json::json!("limited")).unwrap(), AgentState::Limited);
    }

    #[test]
    fn the_reason_reads_for_people() {
        let u = |state, resets_at_ms| Unavailable {
            state,
            code: FailureClass::UsageLimit,
            title: String::new(),
            resets_at_ms,
            since_ms: 0,
            account_id: None,
        };
        assert_eq!(u(AgentState::Limited, None).reason(), "Out of quota");
        assert!(u(AgentState::Limited, Some(1_792_886_400_000)).reason().starts_with("Out of quota until "));
        assert_eq!(u(AgentState::SignedOut, None).reason(), "Needs sign-in");
    }

    fn store() -> Store {
        Store::open_in_memory().expect("in-memory store")
    }

    fn failure_meta(code: &str, resets_at_ms: Option<u64>) -> serde_json::Value {
        let mut f = serde_json::json!({"code": code, "title": "Usage limit reached", "detail": "", "retryable": false});
        if let Some(r) = resets_at_ms {
            f["resetsAtMs"] = serde_json::json!(r);
        }
        f
    }

    #[test]
    fn unavailable_of_block_reads_the_stored_failure_and_the_run_account() {
        let store = store();
        let now = agentmux_common::time::now_ms_u64();
        let limited = insert_block(
            &store,
            &[
                (blockcontroller::core::META_LAST_FAILURE, failure_meta("usage_limit", Some(now + 60_000))),
                (RUN_ACCOUNT_META_KEY, serde_json::json!("acct-1")),
            ],
        );
        test_support::register_stub(&limited, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
        let u = unavailable_of_block(&store, None, &limited, now).expect("limited");
        assert_eq!((u.state, u.code, u.resets_at_ms), (AgentState::Limited, FailureClass::UsageLimit, Some(now + 60_000)));
        assert_eq!(u.title, "Usage limit reached");
        assert_eq!(u.account_id.as_deref(), Some("acct-1"));
        let wire = serde_json::to_value(&u).unwrap();
        assert!(wire.get("account_id").is_none(), "the account never goes on the wire");
        assert_eq!(wire["state"], serde_json::json!("limited"));
        assert_eq!(unavailable_of_block(&store, None, &limited, now + 60_001), None, "the reset passed");

        // A pane from before `agent:run_account` gets its agent's link from the
        // identity store, when one is given; without it, no account.
        let identity = Store::open_in_memory().expect("in-memory identity store");
        let signed_out = insert_block(
            &store,
            &[(blockcontroller::core::META_LAST_FAILURE, failure_meta("auth", None)), ("agentId", serde_json::json!("agent-x"))],
        );
        test_support::register_stub(&signed_out, BLOCK_CONTROLLER_PERSISTENT, STATUS_DONE, false);
        let u = unavailable_of_block(&store, Some(&identity), &signed_out, now).expect("signed out");
        assert_eq!(u.state, AgentState::SignedOut);
        assert_eq!(u.account_id, None, "no link for agent-x");
        assert_eq!(unavailable_of_block(&store, None, &signed_out, now).unwrap().account_id, None);

        let transient = insert_block(&store, &[(blockcontroller::core::META_LAST_FAILURE, failure_meta("rate_limited", None))]);
        test_support::register_stub(&transient, BLOCK_CONTROLLER_PERSISTENT, STATUS_RUNNING, false);
        assert_eq!(unavailable_of_block(&store, None, &transient, now), None, "a transient failure leaves it idle");
        assert_eq!(current_state_at(&store, &transient, now), Some(AgentState::Idle));

        for b in [&limited, &signed_out, &transient] {
            blockcontroller::delete_controller(b);
        }
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
