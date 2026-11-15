// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a block does to the world, which decides what a restart may do with one
//! that was running when the machine stopped
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3, "the step that was in flight").
//!
//! A **read-only** or **idempotent** block is simply run again. An **effectful**
//! one is not run again on its own, because it may already have done its work:
//! the run waits for a person or an agent to say whether to run it again, skip
//! it, or cancel the run (at most once for effects).

use serde_json::Value;

use crate::drone::types::{BlockKind, FlowNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads, or only works on what the run already holds.
    ReadOnly,
    /// Does something, but doing it twice leaves the world as doing it once
    /// (a request that carries an idempotency key).
    Idempotent,
    /// Does something that a second time would repeat: sends, starts, writes.
    Effectful,
}

/// What `node`, a block of `kind`, does. `body` is what a for-each's body block
/// does, and is ignored for any other kind. `resolve` fills in a template with what
/// the run holds now: a key is a key only when it has text once it is filled in.
pub fn effect_of(kind: BlockKind, node: &FlowNode, body: Effect, resolve: &dyn Fn(&str) -> String) -> Effect {
    match kind {
        BlockKind::Condition | BlockKind::Response | BlockKind::Variables | BlockKind::Transform | BlockKind::AppQuery | BlockKind::Wait => Effect::ReadOnly,
        BlockKind::Api => api_effect(node, resolve),
        // An Agent block starts a session that costs money and may act; a Message,
        // a Notify and a Spawn reach someone or start something. A Task is filed
        // again once the task it dedupes against is over, so its `dedupe` key does
        // not make a second run of it harmless.
        BlockKind::Task | BlockKind::Agent | BlockKind::Message | BlockKind::Notify | BlockKind::Spawn => Effect::Effectful,
        // The callee may do anything, and a restart cannot tell how far it got.
        BlockKind::Drone => Effect::Effectful,
        // The items already done would be done again, so it is as the body is.
        BlockKind::ForEach => body,
    }
}

/// A request that only reads is read-only; one that writes is idempotent when it
/// sends an `Idempotency-Key` header that has text once filled in, and effectful
/// when it does not.
fn api_effect(node: &FlowNode, resolve: &dyn Fn(&str) -> String) -> Effect {
    let method = node.data.get("method").and_then(Value::as_str).unwrap_or("GET").to_uppercase();
    if matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS") {
        return Effect::ReadOnly;
    }
    let keyed = node
        .data
        .get("headers")
        .and_then(Value::as_object)
        .is_some_and(|h| h.iter().any(|(name, v)| name.eq_ignore_ascii_case("idempotency-key") && v.as_str().is_some_and(|t| !resolve(t).trim().is_empty())));
    if keyed {
        Effect::Idempotent
    } else {
        Effect::Effectful
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::types::NodePosition;
    use serde_json::json;

    fn node(data: Value) -> FlowNode {
        FlowNode { id: "n".into(), position: NodePosition::default(), data, node_type: None }
    }

    fn of(kind: BlockKind, data: Value) -> Effect {
        effect_of(kind, &node(data), Effect::ReadOnly, &|t| t.to_string())
    }

    #[test]
    fn the_blocks_that_only_work_on_the_runs_own_state_or_read_are_read_only() {
        for kind in [BlockKind::Condition, BlockKind::Response, BlockKind::Variables, BlockKind::Transform, BlockKind::AppQuery] {
            assert_eq!(of(kind, json!({})), Effect::ReadOnly, "{kind:?}");
        }
    }

    #[test]
    fn a_request_is_read_only_to_read_and_effectful_to_write_unless_it_carries_a_key() {
        for m in ["GET", "get", "HEAD", "OPTIONS"] {
            assert_eq!(of(BlockKind::Api, json!({ "method": m })), Effect::ReadOnly, "{m}");
        }
        assert_eq!(of(BlockKind::Api, json!({})), Effect::ReadOnly, "no method is a GET");
        for m in ["POST", "put", "PATCH", "DELETE"] {
            assert_eq!(of(BlockKind::Api, json!({ "method": m })), Effect::Effectful, "{m}");
        }
        let keyed = json!({ "method": "POST", "headers": { "idempotency-key": "{{trigger.id}}" } });
        assert_eq!(of(BlockKind::Api, keyed), Effect::Idempotent);
        let empty = json!({ "method": "POST", "headers": { "Idempotency-Key": "  " } });
        assert_eq!(of(BlockKind::Api, empty), Effect::Effectful, "a blank key is no key");
    }

    #[test]
    fn what_sends_starts_or_costs_is_effectful_and_so_is_a_task_whatever_its_dedupe_key() {
        for kind in [BlockKind::Agent, BlockKind::Message, BlockKind::Notify, BlockKind::Spawn] {
            assert_eq!(of(kind, json!({})), Effect::Effectful, "{kind:?}");
        }
        assert_eq!(of(BlockKind::Task, json!({ "title": "x" })), Effect::Effectful);
        // A dedupe key reuses only a task that is still open: one that finished before the restart is filed again.
        assert_eq!(of(BlockKind::Task, json!({ "title": "x", "dedupe": "{{a.id}}" })), Effect::Effectful);
    }

    #[test]
    fn a_key_that_is_empty_once_filled_in_is_no_key() {
        let keyed = node(json!({ "method": "POST", "headers": { "Idempotency-Key": "{{trigger.key}}" } }));
        let with = |key: &'static str| effect_of(BlockKind::Api, &keyed, Effect::ReadOnly, &move |_| key.to_string());
        assert_eq!(with("abc"), Effect::Idempotent);
        assert_eq!(with(""), Effect::Effectful, "the request goes out with an empty key");
        assert_eq!(with("  "), Effect::Effectful);
    }

    #[test]
    fn a_for_each_is_as_its_body_is() {
        for body in [Effect::ReadOnly, Effect::Idempotent, Effect::Effectful] {
            assert_eq!(effect_of(BlockKind::ForEach, &node(json!({})), body, &|t| t.to_string()), body);
        }
    }
}
