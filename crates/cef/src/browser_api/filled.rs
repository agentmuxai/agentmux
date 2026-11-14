// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Marking a field the agent wrote to, so the approval prompt for a click that submits a form never calls
//! such a field "already on the page" (`scripts/field_changed.js`). A page that keeps its fields in step with
//! its own state moves a field's default value along with what was typed, so the default can't say who
//! wrote it. The mark is a plain attribute: every world sees it, and the page's own rendering leaves it alone.
use serde_json::Value;

use super::act::call_on;
use super::cdp::CdpSession;

/// The attribute every write the host makes sets on the field.
pub(super) const ATTR: &str = "data-agentmux-filled";

/// Mark the element `object_id` as written to.
const MARK: &str = r#"function () { if (this.setAttribute) this.setAttribute("data-agentmux-filled", "1"); return true; }"#;

/// Mark whichever element has focus (text typed or a key pressed goes to it).
const MARK_ACTIVE: &str = r#"(() => { const a = document.activeElement; if (a && a.setAttribute) a.setAttribute("data-agentmux-filled", "1"); return true; })()"#;

/// Mark `object_id`, best effort: a page that refuses it only loses the note on the prompt.
pub(super) async fn mark(cdp: &mut CdpSession, object_id: &str) {
    let _ = call_on(cdp, object_id, MARK, vec![]).await;
}

/// Mark `object_id`, then read its state back (`act::READ_BACK`).
pub(super) async fn mark_then_read(cdp: &mut CdpSession, object_id: &str) -> Result<Value, String> {
    mark(cdp, object_id).await;
    call_on(cdp, object_id, super::act::READ_BACK, vec![]).await
}

/// Mark the focused element, best effort.
pub(super) async fn mark_active(cdp: &mut CdpSession) {
    let _ = cdp.call("Runtime.evaluate", serde_json::json!({ "expression": MARK_ACTIVE, "returnByValue": true })).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_set_the_attribute_field_changed_looks_for() {
        let rule = include_str!("scripts/field_changed.js");
        for script in [MARK, MARK_ACTIVE, super::super::act::PREPARE_FILL, super::super::act::SET_VALUE] {
            assert!(script.contains(ATTR), "{script}");
        }
        assert!(rule.contains(ATTR));
        let _ = Value::Null;
    }
}
