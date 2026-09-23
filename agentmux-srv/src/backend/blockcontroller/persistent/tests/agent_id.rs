// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::*;

fn controller() -> PersistentSubprocessController {
    PersistentSubprocessController::new(
        "tab".to_string(),
        "block".to_string(),
        None,
        None,
        None,
        None,
    )
}

/// reagentx P1 on #2697: `agent_id()` was captured once at spawn and
/// never refreshed on re-registration, so a legitimately renamed or
/// reconfigured agent's own messages could be falsely rejected as an
/// identity mismatch by `inject_message_inner`'s recipient-identity
/// check (#2695). `set_agent_id` (called from `handle_reactive_register`
/// on every re-registration) must actually overwrite the captured value,
/// not just the initial spawn-time one.
#[test]
fn set_agent_id_overwrites_a_stale_captured_value() {
    let ctrl = controller();
    assert_eq!(ctrl.agent_id(), None);

    ctrl.set_agent_id(Some("agentx".to_string()));
    assert_eq!(ctrl.agent_id(), Some("agentx".to_string()));

    ctrl.set_agent_id(Some("agenty".to_string()));
    assert_eq!(ctrl.agent_id(), Some("agenty".to_string()));

    ctrl.set_agent_id(None);
    assert_eq!(ctrl.agent_id(), None);
}
