// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agent.input-history*` and `agent.input-draft`: what the user typed to an
//! agent, kept with the agent (`storage/agent_input_history.rs`,
//! `docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md`). For the UI: the
//! composer's ArrowUp recall and draft. No agent tool reads it.

use super::*;
use crate::backend::storage::agent_input_history::ForgottenBy;

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

/// Agents whose history is being filled from their past conversations, so a
/// second composer opening meanwhile doesn't start another fill.
static BACKFILLING: std::sync::Mutex<std::collections::BTreeSet<String>> = std::sync::Mutex::new(std::collections::BTreeSet::new());

/// The first time a composer reads an agent's history, fill it once with what
/// the user typed in the agent's past conversations (spec §4.7), in the
/// background: the composer gets what is kept now, and every window hears
/// `backfill` when the older entries are in. A history index still being
/// built, or any failure, leaves it to the next read.
fn maybe_backfill(state: &AppState, agent_id: &str) {
    let max = kept_per_agent(state);
    if max == 0 || state.mstore.input_history_backfilled(agent_id).unwrap_or(true) {
        return;
    }
    if !BACKFILLING.lock().unwrap_or_else(|e| e.into_inner()).insert(agent_id.to_string()) {
        return;
    }
    let (state, agent_id) = (state.clone(), agent_id.to_string());
    tokio::spawn(async move {
        let (s, id) = (state.clone(), agent_id.clone());
        let filled = tokio::task::spawn_blocking(move || -> Result<usize, String> {
            let Some(row) = s.mstore.agent_def_get(&id).map_err(|e| e.to_string())? else { return Ok(0) };
            let owner = crate::backend::history::HistoryOwner::of_row(&row);
            let typed = s.history_service.typed_by_user(&owner, max).map_err(|e| e.to_string())?;
            s.mstore
                .input_history_backfill(&id, &typed, max, agentmux_common::time::now_ms())
                .map_err(|e| e.to_string())
        })
        .await;
        BACKFILLING.lock().unwrap_or_else(|e| e.into_inner()).remove(&agent_id);
        match filled {
            Ok(Ok(0)) => {}
            Ok(Ok(n)) => {
                tracing::info!(agent_id = %agent_id, entries = n, "input history: filled from past conversations");
                changed(&state, &agent_id, "backfill");
            }
            Ok(Err(e)) => tracing::warn!(agent_id = %agent_id, error = %e, "input history: not filled yet; the next read tries again"),
            Err(e) => tracing::warn!(agent_id = %agent_id, error = %e, "input history: the fill stopped"),
        }
    });
}

/// Who forgot, from the connection: the agent's slug when it registered as
/// one, else the window (`armory-ui`), as Global Memory writes are recorded.
fn forgotten_by(ctx: &RpcContext) -> ForgottenBy<'_> {
    let name = if ctx.agent_id.is_empty() { "armory-ui" } else { ctx.agent_id.as_str() };
    ForgottenBy { name, uid: "" }
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
        async move { input_history(&s, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY_APPEND, move |cmd: CommandInputHistoryAppendData, _ctx| {
        let s = s.clone();
        async move { input_history_append(&s, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY_SEARCH, move |cmd: CommandInputHistorySearchData, _ctx| {
        let s = s.clone();
        async move { input_history_search(&s, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY_DELETE, move |cmd: CommandInputHistoryDeleteData, ctx| {
        let s = s.clone();
        async move { input_history_delete(&s, &ctx, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY_CLEAR, move |cmd: CommandInputHistoryClearData, ctx| {
        let s = s.clone();
        async move { input_history_clear(&s, &ctx, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY_TRASH, move |cmd: CommandInputHistoryTrashData, _ctx| {
        let s = s.clone();
        async move { input_history_trash(&s, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_HISTORY_RESTORE, move |cmd: CommandInputHistoryRestoreData, ctx| {
        let s = s.clone();
        async move { input_history_restore(&s, &ctx, cmd) }
    });

    let s = state.clone();
    engine.register_typed(COMMAND_AGENT_INPUT_DRAFT, move |cmd: CommandInputDraftData, _ctx| {
        let s = s.clone();
        async move { input_draft(&s, cmd) }
    });
}

/// `agent.input-history`: the agent's entries, newest first, the first read filling it from past conversations.
pub(crate) fn input_history(s: &AppState, cmd: CommandInputHistoryData) -> Result<InputHistoryResult, String> {
    if cmd.before_id.is_none() {
        maybe_backfill(s, &cmd.agent_id);
    }
    let limit = cmd.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let entries = s
        .mstore
        .input_history_list(&cmd.agent_id, cmd.before_id, limit)
        .map_err(|e| format!("agent.input-history: {e}"))?;
    Ok(InputHistoryResult { entries: entries.into_iter().map(item).collect() })
}

/// `agent.input-history-append`: keep what the user just sent, unless recording is off.
pub(crate) fn input_history_append(s: &AppState, cmd: CommandInputHistoryAppendData) -> Result<InputHistoryAppendResult, String> {
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
    let max = kept_per_agent(s);
    if max == 0 {
        return Ok(InputHistoryAppendResult { entry: None });
    }
    let saved = s.mstore.input_history_append(&entry, max).map_err(|e| format!("agent.input-history-append: {e}"))?;
    if saved.is_some() {
        changed(s, &cmd.agent_id, "history");
    }
    Ok(InputHistoryAppendResult { entry: saved.map(item) })
}

/// `agent.input-history-search`.
pub(crate) fn input_history_search(s: &AppState, cmd: CommandInputHistorySearchData) -> Result<InputHistoryResult, String> {
    let limit = cmd.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as usize;
    let entries = s
        .mstore
        .input_history_search(cmd.agent_id.as_deref(), &cmd.query, limit)
        .map_err(|e| format!("agent.input-history-search: {e}"))?;
    Ok(InputHistoryResult { entries: entries.into_iter().map(item).collect() })
}

/// `agent.input-history-delete`: forget one entry, into the trash.
pub(crate) fn input_history_delete(
    s: &AppState,
    ctx: &RpcContext,
    cmd: CommandInputHistoryDeleteData,
) -> Result<InputHistoryRemovedResult, String> {
    let by = forgotten_by(ctx);
    let forgotten = s
        .mstore
        .input_history_delete(cmd.id, by, agentmux_common::time::now_ms())
        .map_err(|e| format!("agent.input-history-delete: {e}"))?;
    if let Some((agent, _)) = &forgotten {
        // Forgetting is deliberate (the UI asks twice): the log says it happened and who, never what was typed.
        tracing::info!(agent_id = %agent, entry_id = cmd.id, by = by.name, "input history: an entry was forgotten");
        changed(s, agent, "history");
    }
    Ok(InputHistoryRemovedResult { removed: u32::from(forgotten.is_some()), batch_id: forgotten.map(|(_, batch)| batch) })
}

/// `agent.input-history-clear`: forget an agent's whole history, into the trash.
pub(crate) fn input_history_clear(
    s: &AppState,
    ctx: &RpcContext,
    cmd: CommandInputHistoryClearData,
) -> Result<InputHistoryRemovedResult, String> {
    let by = forgotten_by(ctx);
    let (batch_id, removed) = s
        .mstore
        .input_history_clear(&cmd.agent_id, by, agentmux_common::time::now_ms())
        .map_err(|e| format!("agent.input-history-clear: {e}"))?;
    if removed > 0 {
        tracing::info!(agent_id = %cmd.agent_id, entries = removed, by = by.name, "input history: an agent's history was cleared");
        changed(s, &cmd.agent_id, "history");
    }
    Ok(InputHistoryRemovedResult { removed: removed as u32, batch_id })
}

/// `agent.input-history-trash`: what was forgotten and can still be restored.
pub(crate) fn input_history_trash(s: &AppState, cmd: CommandInputHistoryTrashData) -> Result<InputHistoryTrashResult, String> {
    let batches = s
        .mstore
        .input_history_trash(cmd.agent_id.as_deref(), agentmux_common::time::now_ms())
        .map_err(|e| format!("agent.input-history-trash: {e}"))?;
    Ok(InputHistoryTrashResult {
        batches: batches
            .into_iter()
            .map(|b| InputHistoryTrashItem {
                batch_id: b.batch_id,
                agent_id: b.agent_id,
                deleted_at_ms: b.deleted_at_ms,
                deleted_by: b.deleted_by,
                count: b.count as u32,
                sample: b.sample,
            })
            .collect(),
        retention_ms: crate::backend::storage::agent_input_history::INPUT_HISTORY_TRASH_RETENTION_MS,
    })
}

/// `agent.input-history-restore`: bring a forgotten batch back.
pub(crate) fn input_history_restore(
    s: &AppState,
    ctx: &RpcContext,
    cmd: CommandInputHistoryRestoreData,
) -> Result<InputHistoryRestoredResult, String> {
    let restored = s
        .mstore
        .input_history_restore(&cmd.batch_id, agentmux_common::time::now_ms())
        .map_err(|e| format!("agent.input-history-restore: {e}"))?;
    let Some((agent, count)) = restored else { return Ok(InputHistoryRestoredResult { restored: 0 }) };
    tracing::info!(agent_id = %agent, entries = count, by = forgotten_by(ctx).name, "input history: forgotten entries were restored");
    changed(s, &agent, "restored");
    Ok(InputHistoryRestoredResult { restored: count as u32 })
}

/// `agent.input-draft`: store the composer's draft when one is sent, and answer with what is stored.
pub(crate) fn input_draft(s: &AppState, cmd: CommandInputDraftData) -> Result<InputDraftResult, String> {
    if let Some(text) = &cmd.text {
        let known = s
            .mstore
            .input_draft_set(&cmd.agent_id, text, agentmux_common::time::now_ms())
            .map_err(|e| format!("agent.input-draft: {e}"))?;
        if known {
            changed(s, &cmd.agent_id, "draft");
        }
    }
    let text = s.mstore.input_draft_get(&cmd.agent_id).map_err(|e| format!("agent.input-draft: {e}"))?;
    Ok(InputDraftResult { text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_forgot_is_the_registered_agent_or_else_the_window() {
        let window = RpcContext::default();
        assert_eq!(forgotten_by(&window).name, "armory-ui");
        let agent = RpcContext { agent_id: "korp".into(), ..Default::default() };
        assert_eq!(forgotten_by(&agent).name, "korp");
    }

    #[tokio::test]
    async fn forgetting_through_the_rpcs_is_recorded_and_restorable() {
        let state = crate::server::tests::test_state();
        let (engine, _rx) = WshRpcEngine::new();
        register(&engine, &state);
        state
            .mstore
            .conn()
            .lock()
            .unwrap()
            .execute("INSERT INTO db_agents (id, name, provider) VALUES ('uid-9', 'A', 'claude')", [])
            .unwrap();
        let entry = crate::backend::storage::agent_input_history::NewInputHistoryEntry {
            agent_id: "uid-9",
            sent_at_ms: agentmux_common::time::now_ms(),
            text: "ship it",
            kind: "message",
            source: "composer",
            ..Default::default()
        };
        state.mstore.input_history_append(&entry, 100).unwrap();
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        for (cmd, req, resp) in [
            (COMMAND_AGENT_INPUT_HISTORY_TRASH, "CommandInputHistoryTrashData", "InputHistoryTrashResult"),
            (COMMAND_AGENT_INPUT_HISTORY_RESTORE, "CommandInputHistoryRestoreData", "InputHistoryRestoredResult"),
        ] {
            let row = rows.iter().find(|r| r["command"] == cmd).unwrap_or_else(|| panic!("{cmd} missing from the schema"));
            assert_eq!((row["requestName"].as_str(), row["responseName"].as_str()), (Some(req), Some(resp)));
        }
        let forgotten = state
            .mstore
            .input_history_clear("uid-9", forgotten_by(&RpcContext { agent_id: "korp".into(), ..Default::default() }), agentmux_common::time::now_ms())
            .unwrap();
        assert_eq!(forgotten.1, 1);
        let trash = state.mstore.input_history_trash(Some("uid-9"), agentmux_common::time::now_ms()).unwrap();
        assert_eq!((trash.len(), trash[0].deleted_by.as_str()), (1, "korp"));
    }

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

    async fn state_with_agent(id: &str) -> AppState {
        let state = crate::server::tests::test_state();
        state
            .mstore
            .conn()
            .lock()
            .unwrap()
            .execute("INSERT INTO db_agents (id, name, provider) VALUES (?1, 'A', 'claude')", [id])
            .unwrap();
        state
    }

    fn append_req(agent: &str, text: &str, kind: &str) -> CommandInputHistoryAppendData {
        serde_json::from_value(serde_json::json!({ "agent_id": agent, "text": text, "kind": kind })).unwrap()
    }

    fn list_req(agent: &str) -> CommandInputHistoryData {
        serde_json::from_value(serde_json::json!({ "agent_id": agent })).unwrap()
    }

    #[tokio::test]
    async fn what_was_appended_is_listed_newest_first_and_an_unknown_kind_is_refused() {
        let state = state_with_agent("uid-h1").await;
        input_history_append(&state, append_req("uid-h1", "first", "message")).unwrap();
        let saved = input_history_append(&state, append_req("uid-h1", "/clear", "slash")).unwrap();
        assert_eq!(saved.entry.unwrap().text, "/clear");
        let listed = input_history(&state, list_req("uid-h1")).unwrap();
        assert_eq!(listed.entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), vec!["/clear", "first"]);

        let err = input_history_append(&state, append_req("uid-h1", "x", "shout")).err().unwrap();
        assert_eq!(err, "agent.input-history-append: unknown kind \"shout\"");
    }

    #[tokio::test]
    async fn a_deleted_entry_goes_to_the_trash_and_comes_back_on_restore() {
        let state = state_with_agent("uid-h2").await;
        let id = input_history_append(&state, append_req("uid-h2", "keep me", "message")).unwrap().entry.unwrap().id;
        let window = RpcContext::default();

        let removed = input_history_delete(&state, &window, CommandInputHistoryDeleteData { id }).unwrap();
        assert_eq!(removed.removed, 1);
        assert!(input_history(&state, list_req("uid-h2")).unwrap().entries.is_empty());
        let trash = input_history_trash(&state, CommandInputHistoryTrashData { agent_id: Some("uid-h2".into()) }).unwrap();
        assert_eq!((trash.batches.len(), trash.batches[0].deleted_by.as_str()), (1, "armory-ui"));

        let restored = input_history_restore(&state, &window, CommandInputHistoryRestoreData { batch_id: removed.batch_id.unwrap() }).unwrap();
        assert_eq!(restored.restored, 1);
        assert_eq!(input_history(&state, list_req("uid-h2")).unwrap().entries.len(), 1);
    }

    #[tokio::test]
    async fn clearing_records_which_agent_did_it_and_clearing_nothing_changes_nothing() {
        let state = state_with_agent("uid-h3").await;
        let cleared_empty = input_history_clear(&state, &RpcContext::default(), CommandInputHistoryClearData { agent_id: "uid-h3".into() }).unwrap();
        assert_eq!(cleared_empty.removed, 0);

        input_history_append(&state, append_req("uid-h3", "a", "message")).unwrap();
        input_history_append(&state, append_req("uid-h3", "b", "message")).unwrap();
        let korp = RpcContext { agent_id: "korp".into(), ..Default::default() };
        let cleared = input_history_clear(&state, &korp, CommandInputHistoryClearData { agent_id: "uid-h3".into() }).unwrap();
        assert_eq!(cleared.removed, 2);
        let trash = input_history_trash(&state, CommandInputHistoryTrashData { agent_id: Some("uid-h3".into()) }).unwrap();
        assert_eq!(trash.batches[0].deleted_by, "korp");
    }

    #[tokio::test]
    async fn a_draft_reads_back_what_was_stored_and_no_text_only_reads() {
        let state = state_with_agent("uid-h4").await;
        let set = |text: Option<&str>| CommandInputDraftData { agent_id: "uid-h4".into(), text: text.map(String::from) };
        assert_eq!(input_draft(&state, set(None)).unwrap().text.as_deref().unwrap_or(""), "");
        assert_eq!(input_draft(&state, set(Some("half a thought"))).unwrap().text.as_deref(), Some("half a thought"));
        assert_eq!(input_draft(&state, set(None)).unwrap().text.as_deref(), Some("half a thought"), "reading leaves it");
    }
}
