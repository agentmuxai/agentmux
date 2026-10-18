// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agent.input-history*` and `agent.input-draft`: what the user typed to an
//! agent, kept with the agent (`storage/agent_input_history.rs`,
//! `docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md`). For the UI: the
//! composer's ArrowUp recall and draft. No agent tool reads it.

use super::*;

const DEFAULT_LIMIT: u32 = 200;
const MAX_LIMIT: u32 = 500;
const KINDS: [&str; 3] = ["message", "slash", "bang"];

fn item(e: crate::backend::storage::agent_input_history::InputHistoryEntry) -> InputHistoryItem {
    InputHistoryItem {
        id: e.id,
        agent_id: e.agent_id,
        sent_at_ms: e.sent_at_ms,
        text: e.text,
        kind: e.kind,
        source: e.source,
        sends: e.sends,
    }
}

/// How many entries each agent keeps, newest first (`agent:inputhistory:max`,
/// 10,000 unless set); 0 turns recording off.
fn kept_per_agent(state: &AppState) -> usize {
    let extra = state.config_watcher.get_settings().extra;
    extra.get("agent:inputhistory:max").and_then(|v| v.as_u64()).unwrap_or(10000) as usize
}

/// Tell every window `agent_id`'s history or draft changed.
fn changed(state: &AppState, agent_id: &str, what: &str) {
    state.broker.publish_event::<crate::backend::mps_events::AgentInputHistoryChanged>(
        &InputHistoryChangedEvent { agent_id: agent_id.to_string(), what: what.to_string() },
        vec![],
        0,
    );
}

/// A Swarm broadcast the user typed reached the agent in `block_id`: keep it
/// in that agent's history (`source: swarm`), as if typed in its composer.
/// A pane with no defined agent keeps nothing.
pub(crate) fn record_broadcast(state: &AppState, block_id: &str, text: &str) {
    let max = kept_per_agent(state);
    let Some(agent_id) = state
        .mstore
        .get::<crate::backend::obj::Block>(block_id)
        .ok()
        .flatten()
        .and_then(|b| crate::backend::obj::block_meta_agent_id(&b.meta).map(|id| id.trim().to_string()))
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    if max == 0 {
        return;
    }
    let entry = crate::backend::storage::agent_input_history::NewInputHistoryEntry {
        agent_id: &agent_id,
        sent_at_ms: agentmux_common::time::now_ms(),
        text,
        kind: "message",
        source: "swarm",
        session_id: None,
        attachments: None,
    };
    match state.mstore.input_history_append(&entry, max) {
        Ok(Some(_)) => changed(state, &agent_id, "history"),
        Ok(None) => {}
        Err(e) => tracing::warn!(block_id, error = %e, "input history: couldn't keep a broadcast"),
    }
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY, move |cmd: CommandInputHistoryData, _ctx| {
        let s = s.clone();
        async move {
            let limit = cmd.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let entries = s
                .mstore
                .input_history_list(&cmd.agent_id, cmd.before_id, limit)
                .map_err(|e| format!("agent.input-history: {e}"))?;
            Ok(InputHistoryResult { entries: entries.into_iter().map(item).collect() })
        }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY_APPEND, move |cmd: CommandInputHistoryAppendData, _ctx| {
        let s = s.clone();
        async move {
            if !KINDS.contains(&cmd.kind.as_str()) {
                return Err(format!("agent.input-history-append: unknown kind {:?}", cmd.kind));
            }
            let source = cmd.source.as_deref().filter(|s| *s == "swarm").unwrap_or("composer");
            let attachments = (!cmd.attachments.is_empty())
                .then(|| serde_json::to_string(&cmd.attachments.iter().map(|n| serde_json::json!({ "name": n })).collect::<Vec<_>>()).ok())
                .flatten();
            let entry = crate::backend::storage::agent_input_history::NewInputHistoryEntry {
                agent_id: &cmd.agent_id,
                sent_at_ms: agentmux_common::time::now_ms(),
                text: &cmd.text,
                kind: &cmd.kind,
                source,
                session_id: None,
                attachments: attachments.as_deref(),
            };
            let max = kept_per_agent(&s);
            if max == 0 {
                return Ok(InputHistoryAppendResult { entry: None });
            }
            let saved = s.mstore.input_history_append(&entry, max).map_err(|e| format!("agent.input-history-append: {e}"))?;
            if saved.is_some() {
                changed(&s, &cmd.agent_id, "history");
            }
            Ok(InputHistoryAppendResult { entry: saved.map(item) })
        }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY_SEARCH, move |cmd: CommandInputHistorySearchData, _ctx| {
        let s = s.clone();
        async move {
            let limit = cmd.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
            let entries = s
                .mstore
                .input_history_search(cmd.agent_id.as_deref(), &cmd.query, limit)
                .map_err(|e| format!("agent.input-history-search: {e}"))?;
            Ok(InputHistoryResult { entries: entries.into_iter().map(item).collect() })
        }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY_DELETE, move |cmd: CommandInputHistoryDeleteData, _ctx| {
        let s = s.clone();
        async move {
            let agent = s.mstore.input_history_delete(cmd.id).map_err(|e| format!("agent.input-history-delete: {e}"))?;
            if let Some(agent) = &agent {
                changed(&s, agent, "history");
            }
            Ok(InputHistoryRemovedResult { removed: u32::from(agent.is_some()) })
        }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY_CLEAR, move |cmd: CommandInputHistoryClearData, _ctx| {
        let s = s.clone();
        async move {
            let removed = s.mstore.input_history_clear(&cmd.agent_id).map_err(|e| format!("agent.input-history-clear: {e}"))?;
            if removed > 0 {
                changed(&s, &cmd.agent_id, "history");
            }
            Ok(InputHistoryRemovedResult { removed: removed as u32 })
        }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_DRAFT, move |cmd: CommandInputDraftData, _ctx| {
        let s = s.clone();
        async move {
            if let Some(text) = &cmd.text {
                let known = s
                    .mstore
                    .input_draft_set(&cmd.agent_id, text, agentmux_common::time::now_ms())
                    .map_err(|e| format!("agent.input-draft: {e}"))?;
                if known {
                    changed(&s, &cmd.agent_id, "draft");
                }
            }
            let text = s.mstore.input_draft_get(&cmd.agent_id).map_err(|e| format!("agent.input-draft: {e}"))?;
            Ok(InputDraftResult { text })
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_swarm_broadcast_is_kept_in_the_recipient_agents_history() {
        let state = crate::server::tests::test_state();
        state
            .mstore
            .conn()
            .lock()
            .unwrap()
            .execute("INSERT INTO db_agents (id, name, provider) VALUES ('uid-1', 'A', 'claude')", [])
            .unwrap();
        let block = |oid: &str, agent: Option<&str>| {
            let mut b = crate::backend::obj::Block { oid: oid.into(), ..Default::default() };
            if let Some(a) = agent {
                b.meta.insert("agentId".into(), serde_json::json!(a));
            }
            state.mstore.insert(&mut b).unwrap();
        };
        block("b-agent", Some(" uid-1 "));
        {
            // The legacy key is read too.
            let mut b = crate::backend::obj::Block { oid: "b-legacy".into(), ..Default::default() };
            b.meta.insert("agent:id".into(), serde_json::json!("uid-1"));
            state.mstore.insert(&mut b).unwrap();
        }
        block("b-quick", Some("claude"));
        block("b-shell", None);
        for b in ["b-agent", "b-quick", "b-shell"] {
            record_broadcast(&state, b, "merge on approval");
        }
        record_broadcast(&state, "b-legacy", "and then release");
        let kept = state.mstore.input_history_list("uid-1", None, 10).unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].text, "and then release");
        let kept = &kept[1..];
        assert_eq!((kept[0].text.as_str(), kept[0].source.as_str()), ("merge on approval", "swarm"));
        assert!(state.mstore.input_history_list("claude", None, 10).unwrap().is_empty(), "a provider key isn't an agent");
        assert_eq!(kept_per_agent(&state), 10000, "the default");
    }
}
