// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests for the block-controller registry and its shared helpers (`super`).

use super::*;

// ── Deferred-restart guard (reagent P2 on PR #2858) ─────────────────

/// The case the deferral exists for: /model, /effort, /permission on a
/// pane that stays persistent.
#[test]
fn a_forced_persistent_to_persistent_replace_may_be_deferred() {
    assert!(is_runtime_config_only_replace(
        BLOCK_CONTROLLER_PERSISTENT,
        BLOCK_CONTROLLER_PERSISTENT,
        true
    ));
}

/// The bug: the container-agent override rewrites the TARGET persistent ->
/// subprocess. Deferring here would keep a persistent controller on a
/// container agent, which cannot do per-turn `docker exec` — the exact
/// incompatibility that override exists to fix.
#[test]
fn a_type_change_to_subprocess_is_never_deferred_even_when_forced() {
    assert!(!is_runtime_config_only_replace(
        BLOCK_CONTROLLER_PERSISTENT,
        BLOCK_CONTROLLER_SUBPROCESS,
        true
    ));
}

#[test]
fn a_type_change_from_a_non_persistent_controller_is_never_deferred() {
    assert!(!is_runtime_config_only_replace(
        BLOCK_CONTROLLER_SUBPROCESS,
        BLOCK_CONTROLLER_PERSISTENT,
        true
    ));
    assert!(!is_runtime_config_only_replace(
        BLOCK_CONTROLLER_SHELL,
        BLOCK_CONTROLLER_SHELL,
        true
    ));
}

/// Without `force` the replace is driven by a type or connection change,
/// never by a runtime-config tweak — nothing to defer.
#[test]
fn an_unforced_replace_is_never_deferred() {
    assert!(!is_runtime_config_only_replace(
        BLOCK_CONTROLLER_PERSISTENT,
        BLOCK_CONTROLLER_PERSISTENT,
        false
    ));
}

/// Test double for the stop_for_replace/remove_controller_entry_only
/// tests below. Counts calls to `stop()` vs `stop_for_replace()`
/// separately so a test can assert exactly one of them fired.
struct CountingController {
    block_id: String,
    controller_type: String,
    stop_calls: std::sync::atomic::AtomicU32,
    stop_for_replace_calls: std::sync::atomic::AtomicU32,
}

impl CountingController {
    fn new(block_id: &str, controller_type: &str) -> Self {
        Self {
            block_id: block_id.to_string(),
            controller_type: controller_type.to_string(),
            stop_calls: std::sync::atomic::AtomicU32::new(0),
            stop_for_replace_calls: std::sync::atomic::AtomicU32::new(0),
        }
    }
}

impl Controller for CountingController {
    fn start(
        &self,
        _: MetaMapType,
        _: Option<serde_json::Value>,
        _: bool,
    ) -> Result<(), String> {
        Ok(())
    }
    fn stop(&self, _graceful: bool, _new_status: &str) -> Result<(), String> {
        self.stop_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
        BlockControllerRuntimeStatus {
            blockid: self.block_id.clone(),
            ..Default::default()
        }
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

/// A second test double that overrides `stop_for_replace` (mirroring
/// `ShellController`'s real override) so the "which method actually
/// fired" assertion below is meaningful — `CountingController` alone
/// would pass even if `resync_controller` still called plain `stop()`,
/// since the DEFAULT `stop_for_replace` delegates to `stop()` too.
struct OverridingCountingController(CountingController);

impl Controller for OverridingCountingController {
    fn start(
        &self,
        m: MetaMapType,
        o: Option<serde_json::Value>,
        f: bool,
    ) -> Result<(), String> {
        self.0.start(m, o, f)
    }
    fn stop(&self, g: bool, s: &str) -> Result<(), String> {
        self.0.stop(g, s)
    }
    fn stop_for_replace(&self, new_status: &str) -> Result<(), String> {
        self.0
            .stop_for_replace_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = new_status;
        Ok(())
    }
    fn get_runtime_status(&self) -> BlockControllerRuntimeStatus {
        self.0.get_runtime_status()
    }
    fn send_input(&self, i: BlockInputUnion, s: Option<u64>) -> Result<(), String> {
        self.0.send_input(i, s)
    }
    fn controller_type(&self) -> &str {
        self.0.controller_type()
    }
    fn block_id(&self) -> &str {
        self.0.block_id()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn default_stop_for_replace_delegates_to_stop() {
    let ctrl = CountingController::new("block-default-delegate", "stub");
    ctrl.stop_for_replace(STATUS_DONE).unwrap();
    assert_eq!(ctrl.stop_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn remove_controller_entry_only_removes_the_registry_entry_without_calling_stop() {
    let block_id = "block-remove-entry-only";
    let ctrl = Arc::new(CountingController::new(block_id, "stub"));
    CONTROLLER_REGISTRY
        .write()
        .unwrap()
        .insert(block_id.to_string(), ctrl.clone());
    assert!(get_controller(block_id).is_some());

    remove_controller_entry_only(block_id);

    assert!(
        get_controller(block_id).is_none(),
        "controller must be gone from the registry"
    );
    assert_eq!(
        ctrl.stop_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "remove_controller_entry_only must not itself call stop() — that's the caller's job via stop_for_replace"
    );
}

#[test]
fn resync_controller_replace_path_calls_stop_for_replace_not_stop() {
    use crate::backend::obj::Block;

    let block_id = "block-resync-replace-uses-stop-for-replace";
    let old = Arc::new(OverridingCountingController(CountingController::new(
        block_id, "old-type",
    )));
    register_controller(block_id, old.clone());
    assert!(get_controller(block_id).is_some());

    // A real ShellController with cmd:runonstart=false so resync_controller's
    // replacement construction doesn't open a real PTY — controller_type
    // "shell" != old's "old-type" forces needs_replace=true.
    let mut meta = MetaMapType::new();
    meta.insert(
        META_KEY_CONTROLLER.to_string(),
        serde_json::Value::String("shell".to_string()),
    );
    meta.insert(
        META_KEY_CMD_RUN_ON_START.to_string(),
        serde_json::Value::Bool(false),
    );
    let block = Block {
        oid: block_id.to_string(),
        version: 1,
        meta,
        ..Default::default()
    };

    let result = resync_controller(&block, "tab-1", None, false, true, None, None, None, None, None, None, None, Arc::from("test-boot"), "test-key", None);
    assert!(result.is_ok(), "resync_controller failed: {result:?}");

    assert_eq!(
        old.0
            .stop_for_replace_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the OLD controller's stop_for_replace should have fired exactly once"
    );
    assert_eq!(
        old.0.stop_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the OLD controller's plain stop() must NOT have fired — that would (on ShellController) SIGTERM the whole process group, taking a declared-background descendant down with it"
    );

    // A new controller now owns the block, replacing the old one.
    let replaced = get_controller(block_id);
    assert!(replaced.is_some());
    assert_eq!(replaced.unwrap().controller_type(), "shell");

    // Cleanup — real teardown, not a replace, so the ordinary path is fine.
    delete_controller(block_id);
}

#[test]
fn test_status_constants() {
    assert_eq!(STATUS_INIT, "init");
    assert_eq!(STATUS_RUNNING, "running");
    assert_eq!(STATUS_DONE, "done");
}

#[test]
fn test_controller_type_constants() {
    assert_eq!(BLOCK_CONTROLLER_SHELL, "shell");
    assert_eq!(BLOCK_CONTROLLER_CMD, "cmd");
    assert_eq!(BLOCK_CONTROLLER_TSUNAMI, "tsunami");
    assert_eq!(BLOCK_CONTROLLER_APP_SERVER, "app-server");
}

#[test]
fn test_meta_key_constants() {
    assert_eq!(META_KEY_CONTROLLER, "controller");
    assert_eq!(META_KEY_CONNECTION, "connection");
    assert_eq!(META_KEY_CMD, "cmd");
    assert_eq!(META_KEY_CMD_RUN_ON_START, "cmd:runonstart");
}

#[test]
fn test_block_input_union_data() {
    let input = BlockInputUnion::data(b"hello".to_vec());
    assert_eq!(input.input_data.as_ref().unwrap(), b"hello");
    assert!(input.sig_name.is_none());
    assert!(input.term_size.is_none());
}

#[test]
fn test_block_input_union_signal() {
    let input = BlockInputUnion::signal("SIGTERM");
    assert!(input.input_data.is_none());
    assert_eq!(input.sig_name.as_ref().unwrap(), "SIGTERM");
    assert!(input.term_size.is_none());
}

#[test]
fn test_block_input_union_resize() {
    let size = TermSize {
        rows: 40,
        cols: 120,
    };
    let input = BlockInputUnion::resize(size.clone());
    assert!(input.input_data.is_none());
    assert!(input.sig_name.is_none());
    let ts = input.term_size.unwrap();
    assert_eq!(ts.rows, 40);
    assert_eq!(ts.cols, 120);
}

#[test]
fn test_runtime_status_default() {
    let status = BlockControllerRuntimeStatus::default();
    assert!(status.blockid.is_empty());
    assert_eq!(status.version, 0);
    assert!(status.shellprocstatus.is_empty());
    assert_eq!(status.shellprocexitcode, 0);
}

#[test]
fn test_runtime_status_serde() {
    let status = BlockControllerRuntimeStatus {
        blockid: "block-123".to_string(),
        version: 3,
        shellprocstatus: STATUS_RUNNING.to_string(),
        shellprocconnname: "local".to_string(),
        shellprocexitcode: 0,
        ..Default::default()
    };
    let json = serde_json::to_string(&status).unwrap();
    assert!(json.contains("\"blockid\":\"block-123\""));
    assert!(json.contains("\"shellprocstatus\":\"running\""));

    let parsed: BlockControllerRuntimeStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.blockid, "block-123");
    assert_eq!(parsed.version, 3);
}

#[test]
fn test_get_nonexistent_controller() {
    assert!(get_controller("nonexistent-block").is_none());
}

#[test]
fn test_get_block_controller_status_none() {
    assert!(get_block_controller_status("nonexistent").is_none());
}

#[test]
fn test_stop_nonexistent_controller() {
    // Should be ok (no-op)
    assert!(stop_block_controller("nonexistent").is_ok());
}

#[test]
fn test_send_input_no_controller() {
    let result = send_input("nonexistent", BlockInputUnion::data(b"test".to_vec()), None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no controller"));
}

#[test]
fn test_resync_no_controller_type() {
    let block = Block {
        oid: "test-block".to_string(),
        version: 1,
        meta: HashMap::new(),
        ..Default::default()
    };
    // No "controller" key in meta = no-op
    let result = resync_controller(&block, "tab-1", None, false, true, None, None, None, None, None, None, None, std::sync::Arc::from("test-boot"), "test-key", None);
    assert!(result.is_ok());
}

#[test]
fn test_resync_unknown_controller_type() {
    let mut meta = MetaMapType::new();
    meta.insert(
        "controller".to_string(),
        serde_json::Value::String("unknown_type".to_string()),
    );
    let block = Block {
        oid: "test-block".to_string(),
        version: 1,
        meta,
        ..Default::default()
    };
    let result = resync_controller(&block, "tab-1", None, false, true, None, None, None, None, None, None, None, std::sync::Arc::from("test-boot"), "test-key", None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unknown controller type"));
}

/// Regression: `publish_controller_status` must persist (not fire-once),
/// so a reconnecting subscriber picks up the last known `turn_active`
/// state instead of nothing — see this function's doc comment and
/// REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md §3/§4
/// item 5.
#[test]
fn test_publish_controller_status_persists_for_replay() {
    let broker = super::super::mps::Broker::new();
    let status = BlockControllerRuntimeStatus {
        blockid: "block-persist-test".to_string(),
        turn_active: true,
        ..Default::default()
    };
    publish_controller_status(&broker, &status);

    let history = broker.read_event_history(
        super::super::mps::EVENT_CONTROLLER_STATUS,
        "block:block-persist-test",
        1,
    );
    assert_eq!(
        history.len(),
        1,
        "publish must persist at least the latest event"
    );
    let replayed: BlockControllerRuntimeStatus =
        serde_json::from_value(history[0].data.clone().unwrap()).unwrap();
    assert_eq!(replayed.blockid, "block-persist-test");
    assert!(replayed.turn_active);
}

#[test]
fn agent_runtime_status_sets_the_fixed_agent_pane_fields() {
    let status = agent_runtime_status("block-a", 7, STATUS_DONE, -1, true);
    assert_eq!(status.blockid, "block-a");
    assert_eq!(status.version, 7);
    assert_eq!(status.shellprocstatus, STATUS_DONE);
    assert_eq!(status.shellprocconnname, "local");
    assert_eq!(status.shellprocexitcode, -1);
    assert_eq!(status.shellprocpid, None);
    assert!(status.shellprocname.is_empty());
    assert_eq!(status.spawn_ts_ms, None);
    assert!(status.is_agent_pane);
    assert!(status.turn_active);
}

// ── Subprocess agents are unreachable via this primitive alone ──────────
// Regression lock for
// docs/reports/REPORT_JEKT_DELIVERY_DROPS_SUBPROCESS_AGENTS_2026_09_02.md.
// These two tests together are the whole bug: `deliver_agent_message` hands
// a SubprocessController to the PTY fallback, and a SubprocessController
// refuses PTY input — so every inter-agent message to one was dropped.
// `bootstrap::install_agent_turn_delivery` is what closes the gap, by
// running a real turn instead of falling through to keystrokes.

fn subprocess_controller(block_id: &str) -> Arc<subprocess::SubprocessController> {
    Arc::new(subprocess::SubprocessController::new(
        "tab-jekt".to_string(),
        block_id.to_string(),
        None,
        None,
        None,
        None,
        None,
        Arc::from("test-boot"),
    ))
}

/// Half one: this primitive has no structured route to a subprocess agent.
/// If someone later teaches it one, this test should be updated together
/// with the bootstrap installer — not deleted on its own.
#[test]
fn deliver_agent_message_has_no_structured_route_to_a_subprocess_agent() {
    let block_id = "block-jekt-subprocess-delivery";
    register_controller(block_id, subprocess_controller(block_id));

    let delivery = deliver_agent_message(block_id, "hello from another agent")
        .expect("controller is registered, so lookup must succeed");

    assert!(
        matches!(delivery, AgentDelivery::Pty),
        "a SubprocessController still falls back to PTY here; the turn-based              route lives in bootstrap::install_agent_turn_delivery",
    );

    remove_controller_entry_only(block_id);
}

/// Half two: and that fallback is not merely suboptimal — it is refused, so
/// the message reaches the agent not at all rather than late.
#[test]
fn and_the_pty_fallback_that_implies_is_refused_outright() {
    let ctrl = subprocess_controller("block-jekt-subprocess-refusal");

    let err = ctrl
        .send_input(
            BlockInputUnion::data(b"hello from another agent".to_vec()),
            None,
        )
        .expect_err("subprocess controllers take turns, not keystrokes");

    assert!(
        err.contains("does not accept raw input"),
        "expected the raw-input refusal, got {err:?}",
    );
}

// ── App Server agents have no PTY either ─────────────────────────────────
// ReAgent P1 on PR #3215: `deliver_agent_message` only special-cased
// Persistent and ACP, so an App Server block fell through to the PTY
// branch — the same class of bug as the subprocess case above, just for a
// controller that didn't exist yet when that fix landed.

#[test]
fn deliver_agent_message_routes_to_the_app_server_controller_not_pty() {
    let block_id = "block-jekt-app-server-delivery";
    register_controller(
        block_id,
        Arc::new(app_server_controller::AppServerController::new(
            "tab-jekt".to_string(),
            block_id.to_string(),
            None,
            None,
            None,
            None,
        )),
    );

    let err = deliver_agent_message(block_id, "hello from another agent")
        .expect_err("controller has no process yet, so send_message must fail — the point is that it was called at all");

    assert!(
        err.contains("not initialized"),
        "expected AppServerController::send_message's own error (proving the \
         Structured route was taken), got {err:?}",
    );

    remove_controller_entry_only(block_id);
}

/// Controllers that DO have a structured channel must keep the behavior they
/// had — the fix adds a branch, it does not reroute persistent/ACP agents.
#[test]
fn a_missing_controller_is_still_an_error_not_a_silent_pty_fallback() {
    let err = deliver_agent_message("block-that-was-never-registered", "hi")
        .expect_err("an unregistered block has nowhere to deliver to");
    assert!(err.contains("no controller for block"), "got {err:?}");
}
