// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The named steps of `listrecentsessions` (`recent.rs`) and the session zone functions (`zone.rs`), called
//! directly: no RPC engine, and for the steps that read nothing, no store.

use std::path::Path;

use super::recent::*;
use super::zone;
use crate::backend::agent_session::history_summary::{AgentHistory, HistoryRead};
use crate::backend::rpc_types::{
    CommandAgentSessionAppendOutputData, CommandAgentSessionArchiveData, CommandAgentSessionListArchivesData,
    CommandAgentSessionReadData, CommandAgentSessionWriteStateData, RecentSessionRow, RpcContext,
};
use crate::backend::storage::store::{AgentInstance, Bundle};
use crate::registry::{NamedAgentRecord, NamedAgentRecordV1};
use crate::server::tests::test_state;

fn record(instance_id: &str, definition_id: &str, name: &str, launched_ms: i64, identity: Option<&str>) -> NamedAgentRecord {
    NamedAgentRecord {
        schema_version: 1,
        data: NamedAgentRecordV1 {
            instance_id: instance_id.to_string(),
            instance_name: name.to_string(),
            definition_id: definition_id.to_string(),
            identity_id: identity.map(str::to_string),
            memory_id: None,
            session_id: None,
            working_dir: "work".to_string(),
            source_agents_base: None,
            created_at_ms: 1,
            last_launched_at_ms: launched_ms,
            created_by_version: "t".to_string(),
            last_launched_by_version: "t".to_string(),
        },
    }
}

fn instance(definition_id: &str, name: &str, session_id: &str, started_at: i64) -> AgentInstance {
    AgentInstance {
        id: format!("{definition_id}-{name}"),
        definition_id: definition_id.to_string(),
        parent_instance_id: String::new(),
        block_id: String::new(),
        session_id: session_id.to_string(),
        status: "available".to_string(),
        github_context: String::new(),
        started_at,
        ended_at: 0,
        created_at: 1,
        identity_id: String::new(),
        memory_id: String::new(),
        instance_name: name.to_string(),
        working_directory: String::new(),
        display_hidden: false,
    }
}

#[test]
fn the_limit_defaults_to_twenty_is_capped_at_a_hundred_and_pulls_ten_times_as_many_candidates() {
    assert_eq!(request_limits(0), (20, 200));
    assert_eq!(request_limits(3), (3, 50));
    assert_eq!(request_limits(7), (7, 70));
    assert_eq!(request_limits(100), (100, 500));
    assert_eq!(request_limits(5000), (100, 500));
}

#[test]
fn the_registry_records_are_filtered_deduped_ordered_and_cut() {
    let records = vec![
        record("i1", "d1", "a", 100, Some("acct")),
        record("i2", "d1", "a", 400, Some("acct")),
        record("i3", "d2", "b", 300, Some("other")),
        record("i4", "d3", "c", 200, Some("acct")),
    ];
    let order = |r: &[NamedAgentRecord]| r.iter().map(|x| x.data.instance_id.clone()).collect::<Vec<_>>();

    assert_eq!(order(&newest_launch_per_agent(records.clone(), None, 10)), ["i2", "i3", "i4"]);
    assert_eq!(order(&newest_launch_per_agent(records.clone(), Some("acct"), 10)), ["i2", "i4"]);
    assert_eq!(order(&newest_launch_per_agent(records, None, 2)), ["i2", "i3"]);
}

#[test]
fn of_two_local_instances_of_one_agent_the_newer_is_kept() {
    let by_key = newest_local_by_key(vec![
        instance("d1", "a", "old", 10),
        instance("d1", "a", "new", 20),
        instance("d1", "a", "older", 5),
        instance("d2", "a", "other", 1),
    ]);
    assert_eq!(by_key.len(), 2);
    assert_eq!(by_key[&("d1".to_string(), "a".to_string())].session_id, "new");
}

#[test]
fn a_local_rows_empty_session_id_is_filled_only_from_a_non_empty_registry_one() {
    let with_sid = |sid: &str| instance("d1", "a", sid, 1);
    assert_eq!(with_registry_session_id(&with_sid(""), Some("reg")).session_id, "reg");
    assert_eq!(with_registry_session_id(&with_sid(""), Some("")).session_id, "");
    assert_eq!(with_registry_session_id(&with_sid(""), None).session_id, "");
    assert_eq!(with_registry_session_id(&with_sid("mine"), Some("reg")).session_id, "mine");
}

#[test]
fn a_registry_only_agents_workdir_comes_from_its_own_base_then_the_channels_then_the_bare_name() {
    let mut with_base = record("i", "d", "n", 1, None).data;
    with_base.source_agents_base = Some("/other/agents".to_string());
    assert_eq!(synthesized_instance(with_base, Some(Path::new("/chan"))).working_directory, "/other/agents/work");

    let plain = record("i", "d", "n", 1, None).data;
    assert_eq!(synthesized_instance(plain.clone(), Some(Path::new("/chan"))).working_directory, "/chan/work");
    assert_eq!(synthesized_instance(plain, None).working_directory, "work");
}

#[test]
fn a_registry_only_agent_is_available_with_no_block_and_its_launch_time() {
    let mut d = record("inst", "def", "name", 77, Some("acct")).data;
    d.session_id = Some("sid".to_string());
    let inst = synthesized_instance(d, None);
    assert_eq!((inst.id.as_str(), inst.status.as_str(), inst.block_id.as_str()), ("inst", "available", ""));
    assert_eq!((inst.started_at, inst.created_at, inst.ended_at), (77, 1, 0));
    assert_eq!((inst.session_id.as_str(), inst.identity_id.as_str(), inst.memory_id.as_str()), ("sid", "acct", ""));
}

#[test]
fn only_the_local_agents_no_registry_row_covers_are_appended() {
    let mut out = vec![instance("d1", "a", "", 1)];
    let local = newest_local_by_key(vec![instance("d1", "a", "covered", 9), instance("d2", "b", "alone", 2)]);
    append_local_only(&mut out, local);
    assert_eq!(out.len(), 2);
    assert!(out.iter().any(|i| i.definition_id == "d2"));
    assert_eq!(out.iter().find(|i| i.definition_id == "d1").unwrap().session_id, "", "the covered one is not added twice");
}

#[test]
fn a_memory_is_named_vanilla_when_none_is_set_and_missing_when_it_is_gone() {
    let memory = |id: &str, name: &str| -> Bundle {
        serde_json::from_value(serde_json::json!({ "id": id, "name": name })).unwrap()
    };
    let memories = vec![memory("m1", "Rules")];
    let mut inst = instance("d", "n", "", 1);
    assert_eq!(memory_name(&inst, &memories), "(vanilla CLI)");
    inst.memory_id = "m1".to_string();
    assert_eq!(memory_name(&inst, &memories), "Rules");
    inst.memory_id = "gone".to_string();
    assert_eq!(memory_name(&inst, &memories), "(missing memory)");
}

#[test]
fn links_are_grouped_by_agent() {
    let link = |agent: &str, account: &str| {
        serde_json::from_value(serde_json::json!({ "agent_id": agent, "account_id": account, "provider": "claude" })).unwrap()
    };
    let links: Vec<crate::backend::storage::store::AgentIdentityLink> =
        vec![link("a1", "x"), link("a2", "y"), link("a1", "z")];
    let grouped = group_links(&links);
    assert_eq!(grouped["a1"].len(), 2);
    assert_eq!(grouped["a2"].len(), 1);
}

#[tokio::test]
async fn a_transcript_gives_the_row_its_last_prompt_and_time_and_no_node_count() {
    let inst = instance("d", "n", "", 50);
    let history = |ms: i64| HistoryRead::Transcript(AgentHistory { last_user_message: "hi".to_string(), last_activity_ms: ms });
    let store = test_state().filestore;

    let a = activity_from_transcript(&inst, Some(&history(900)), &store).unwrap();
    assert_eq!((a.has_snapshot, a.last_active_at, a.preview.as_str(), a.node_count, a.snapshot_check_failed), (true, 900, "hi", 0, false));
    let no_time = activity_from_transcript(&inst, Some(&history(0)), &store).unwrap();
    assert_eq!(no_time.last_active_at, 50, "no activity time: the launch time stands in");
}

#[tokio::test]
async fn a_failed_transcript_read_is_reported_and_no_transcript_defers_to_the_snapshot() {
    let inst = instance("d", "n", "", 50);
    let store = test_state().filestore;

    let failed = activity_from_transcript(&inst, Some(&HistoryRead::Failed), &store).unwrap();
    assert!(failed.snapshot_check_failed && !failed.has_snapshot);
    assert!(activity_from_transcript(&inst, Some(&HistoryRead::NoTranscript), &store).is_none());
    assert!(activity_from_transcript(&inst, None, &store).is_none());
    assert!(activity_from_transcript(&inst, Some(&HistoryRead::Damaged), &store).is_none(), "damaged with no block of its own has nothing to fall back on");
}

#[tokio::test]
async fn a_damaged_transcript_with_no_copy_under_the_block_is_not_reported_as_history() {
    let mut inst = instance("d", "n", "", 50);
    inst.block_id = "block-with-no-copy".to_string();
    let store = test_state().filestore;
    assert!(activity_from_transcript(&inst, Some(&HistoryRead::Damaged), &store).is_none());
}

#[tokio::test]
async fn a_row_with_no_block_has_no_snapshot_to_read() {
    let inst = instance("d", "n", "", 50);
    let store = test_state().filestore;
    let mut degraded = Vec::new();
    let a = activity_from_snapshot(&inst, &store, &mut degraded);
    assert_eq!((a.has_snapshot, a.last_active_at, a.snapshot_check_failed), (false, 50, false));
    assert!(degraded.is_empty());
}

fn row(name: &str, has_snapshot: bool, last_active_at: i64) -> RecentSessionRow {
    serde_json::from_value(serde_json::json!({
        "instance_id": name, "instance_name": name, "definition_id": "d", "definition_name": "D", "provider": "claude",
        "model_vendor_base_url": "", "working_directory": "", "identity_id": "", "identity_name": "", "memory_id": "",
        "memory_name": "", "block_id_hint": "", "session_id": "", "preview": "", "node_count": 0,
        "last_active_at": last_active_at, "has_snapshot": has_snapshot, "snapshot_check_failed": false,
        "agent_created_at": 0, "started_at": 0, "agent_type": "",
    }))
    .unwrap()
}

#[test]
fn rows_with_a_snapshot_come_first_each_group_newest_activity_first() {
    let mut rows = vec![row("plain-new", false, 900), row("snap-old", true, 100), row("plain-old", false, 50), row("snap-new", true, 300)];
    sort_recent_first(&mut rows);
    let names: Vec<&str> = rows.iter().map(|r| r.instance_name.as_str()).collect();
    assert_eq!(names, ["snap-new", "snap-old", "plain-new", "plain-old"]);
}

// ── The session zone ───────────────────────────────────────────────────────

/// A state with one agent whose slug is `slug`, and its definition id. The transcript store is shared by the
/// test process, so the tag keeps ids apart.
fn zone_agent(tag: &str) -> (crate::server::AppState, String, String) {
    let state = test_state();
    let id = format!("uid-zone-{tag}");
    let slug = format!("agent-zone-{tag}");
    let mut def = crate::backend::storage::agents::test_agent_def(&id, "zone agent", "claude", "host", 1, "");
    def.slug = slug.clone();
    state.mstore.agent_def_insert(&mut def).unwrap();
    (state, id, slug)
}

fn as_agent(slug: &str) -> RpcContext {
    RpcContext { agent_id: slug.to_string(), ..Default::default() }
}

#[tokio::test]
async fn an_agent_writes_and_reads_its_own_session_state_and_another_agent_cannot() {
    let (state, id, slug) = zone_agent("write");
    let write = CommandAgentSessionWriteStateData { definition_id: id.clone(), content: "{\"nodes\":[]}".to_string() };

    let wrote = zone::write_state(&state.filestore, &state.mstore, &as_agent(&slug), &write).unwrap();
    assert_eq!(wrote.bytes_written, 12);
    let read = zone::read_session(&state.filestore, &CommandAgentSessionReadData { definition_id: id }).unwrap();
    assert_eq!(read.content.as_deref(), Some("{\"nodes\":[]}"));

    let err = zone::write_state(&state.filestore, &state.mstore, &as_agent("someone-else"), &write).unwrap_err();
    assert!(err.starts_with("FORBIDDEN:"), "{err}");
    let err = zone::write_state(&state.filestore, &state.mstore, &RpcContext::default(), &write).unwrap_err();
    assert!(err.starts_with("FORBIDDEN:"), "a connection that proved nothing is refused: {err}");
}

#[tokio::test]
async fn the_window_may_write_an_agents_session_state() {
    let (state, id, _slug) = zone_agent("window");
    let write = CommandAgentSessionWriteStateData { definition_id: id, content: "x".to_string() };
    let wrote = zone::write_state(&state.filestore, &state.mstore, &RpcContext::operator_for_tests(), &write).unwrap();
    assert_eq!(wrote.bytes_written, 1);
}

#[tokio::test]
async fn an_append_reports_the_bytes_and_is_refused_for_another_agent() {
    let (state, id, slug) = zone_agent("append");
    let append = CommandAgentSessionAppendOutputData { definition_id: id, line: "{\"type\":\"x\"}".to_string() };
    assert!(zone::append_output(&state.filestore, &state.mstore, &as_agent(&slug), &append).unwrap().bytes_written > 0);
    let err = zone::append_output(&state.filestore, &state.mstore, &as_agent("someone-else"), &append).unwrap_err();
    assert!(err.starts_with("FORBIDDEN:"), "{err}");
}

#[tokio::test]
async fn archiving_nothing_answers_an_empty_zone_and_no_time_and_a_fresh_agent_has_no_archives() {
    let (state, id, slug) = zone_agent("archive");
    let archived = zone::archive(
        &state.filestore,
        &state.mstore,
        &as_agent(&slug),
        &CommandAgentSessionArchiveData { definition_id: id.clone() },
    )
    .unwrap();
    assert_eq!((archived.archive_zoneid.as_str(), archived.archived_at_ms), ("", 0));

    let listed = zone::list_archives(&state.filestore, &CommandAgentSessionListArchivesData { definition_id: id, limit: 10 }).unwrap();
    assert!(listed.is_empty());
}
