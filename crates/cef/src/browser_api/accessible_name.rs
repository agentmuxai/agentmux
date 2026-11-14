// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The accessible name the browser computes for an element: what a snapshot or a find shows for it, and what the
//! commit check (`act::check_click`) reads beside the element's own text and aria-label.

use serde_json::{json, Value};

use super::act::backend_of;
use super::cdp::CdpSession;

/// The accessible name of the element `object_id`. `None` when the browser gave no node to read it from (the
/// caller then treats the click as unnameable), `Some("")` for an element with no name.
pub(super) async fn of(cdp: &mut CdpSession, object_id: &str) -> Option<String> {
    let backend = backend_of(cdp, object_id).await?;
    let tree = cdp.call("Accessibility.getPartialAXTree", json!({ "backendNodeId": backend, "fetchRelatives": false })).await.ok()?;
    in_partial_tree(&tree)
}

/// The name of the first node of a `getPartialAXTree` answer (the element's own).
fn in_partial_tree(tree: &Value) -> Option<String> {
    let node = tree.pointer("/nodes/0")?;
    Some(super::hidden_secrets::scrub(node.pointer("/name/value").and_then(|n| n.as_str()).unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accessible_name_is_the_first_nodes() {
        let named = json!({ "nodes": [{ "name": { "value": "Submit for certification" } }] });
        assert_eq!(in_partial_tree(&named).as_deref(), Some("Submit for certification"));
        // An element with no name is read as unnamed, not as unreadable.
        assert_eq!(in_partial_tree(&json!({ "nodes": [{ "role": { "value": "button" } }] })).as_deref(), Some(""));
        // No node to read it from: unreadable.
        assert_eq!(in_partial_tree(&json!({ "nodes": [] })), None);
        assert_eq!(in_partial_tree(&json!({})), None);
    }
}
