// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The session RPCs: `listrecentsessions` (the My Agents list, `recent.rs`) and the agent-anchored session zone
//! (`zone.rs`). `register` only wires each command to its named function.

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    CommandAgentSessionAppendOutputData, CommandAgentSessionArchiveData, CommandAgentSessionListArchivesData,
    CommandAgentSessionReadData, CommandAgentSessionWriteStateData, COMMAND_AGENT_SESSION_APPEND_OUTPUT,
    COMMAND_AGENT_SESSION_ARCHIVE, COMMAND_AGENT_SESSION_LIST_ARCHIVES, COMMAND_AGENT_SESSION_READ,
    COMMAND_AGENT_SESSION_WRITE_STATE, COMMAND_LIST_RECENT_SESSIONS,
};

use super::super::AppState;

mod recent;
mod zone;

/// At-most-once-per-process-lifetime claim set for definition-summary
/// generation attempts triggered from `listrecentsessions` — see the call
/// site in `recent.rs` and `db_agent_activity_summaries` (OBJECT_SCHEMA_VERSION
/// v28). `HashSet::insert` returns `true` only the first time a given
/// `definition_id` is claimed; every later call for the same id (a repeat
/// poll of "My Agents") sees `false` and does not re-spawn.
fn definition_summary_attempted() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_recent_sessions(engine, state);
    register_session_zone(engine, state);
}

fn register_recent_sessions(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let deps = recent::RecentSessionsDeps {
        mstore: state.mstore.clone(),
        id_store: state.id_store.clone(),
        identity_store: state.identity_store.clone(),
        filestore: state.filestore.clone(),
        broker: state.broker.clone(),
    };
    // `Value` request: a malformed one means the defaults.
    engine.register_typed(COMMAND_LIST_RECENT_SESSIONS, move |data: serde_json::Value, _ctx| {
        let deps = deps.clone();
        async move { recent::list_recent_sessions(&deps, data).await }
    });
}

fn register_session_zone(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let filestore = state.filestore.clone();
    engine.register_typed(COMMAND_AGENT_SESSION_READ, move |cmd: CommandAgentSessionReadData, _ctx| {
        let filestore = filestore.clone();
        async move { zone::read_session(&filestore, &cmd) }
    });

    let (filestore, mstore) = (state.filestore.clone(), state.mstore.clone());
    engine.register_typed(COMMAND_AGENT_SESSION_WRITE_STATE, move |cmd: CommandAgentSessionWriteStateData, ctx| {
        let (filestore, mstore) = (filestore.clone(), mstore.clone());
        async move { zone::write_state(&filestore, &mstore, &ctx, &cmd) }
    });

    let (filestore, mstore) = (state.filestore.clone(), state.mstore.clone());
    engine.register_typed(COMMAND_AGENT_SESSION_APPEND_OUTPUT, move |cmd: CommandAgentSessionAppendOutputData, ctx| {
        let (filestore, mstore) = (filestore.clone(), mstore.clone());
        async move { zone::append_output(&filestore, &mstore, &ctx, &cmd) }
    });

    let (filestore, mstore) = (state.filestore.clone(), state.mstore.clone());
    engine.register_typed(COMMAND_AGENT_SESSION_ARCHIVE, move |cmd: CommandAgentSessionArchiveData, ctx| {
        let (filestore, mstore) = (filestore.clone(), mstore.clone());
        async move { zone::archive(&filestore, &mstore, &ctx, &cmd) }
    });

    let filestore = state.filestore.clone();
    engine.register_typed(COMMAND_AGENT_SESSION_LIST_ARCHIVES, move |cmd: CommandAgentSessionListArchivesData, _ctx| {
        let filestore = filestore.clone();
        async move { zone::list_archives(&filestore, &cmd) }
    });
}

#[cfg(test)]
mod characterization_tests;

#[cfg(test)]
mod direct_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::{AgentDefinition, AgentInstance};
    use crate::backend::rpc_types::{ListRecentSessionsResult, RpcMessage};
    use crate::registry::{
        DefinitionRecord, DefinitionRecordV1, DefinitionStore, NamedAgentRecord,
        NamedAgentRecordV1, Registry,
    };
    use crate::server::tests::test_state;

    fn named_record(instance_id: &str, definition_id: &str) -> NamedAgentRecord {
        NamedAgentRecord {
            schema_version: 1,
            data: NamedAgentRecordV1 {
                instance_id: instance_id.to_string(),
                instance_name: format!("session-{instance_id}"),
                definition_id: definition_id.to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: format!("{definition_id}-workdir"),
                source_agents_base: None,
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "test".to_string(),
                last_launched_by_version: "test".to_string(),
            },
        }
    }

    fn definition_record(id: &str) -> DefinitionRecord {
        DefinitionRecord {
            schema_version: 1,
            data: DefinitionRecordV1 {
                id: id.to_string(),
                name: format!("Agent {id}"),
                provider: "claude".to_string(),
                is_seeded: 0,
                ..Default::default()
            },
        }
    }

    /// Seeds `state.mstore` with N cross-channel registry + definition
    /// records (as they'd exist on a REAL machine before a fresh channel
    /// ever boots) and returns the dispatch machinery ready to call
    /// `listrecentsessions`.
    fn setup_with_n_cross_channel_agents(
        n: usize,
    ) -> (
        AppState,
        std::sync::Arc<WshRpcEngine>,
        tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let state = test_state();
        let registry_dir = tempfile::tempdir().unwrap();
        let def_dir = tempfile::tempdir().unwrap();
        let registry = Registry::open(registry_dir.path().to_path_buf()).unwrap();
        let def_store = DefinitionStore::open(def_dir.path().to_path_buf()).unwrap();
        for i in 0..n {
            let id = format!("inst-{i}");
            let def_id = format!("def-{i}");
            registry.upsert(&named_record(&id, &def_id)).unwrap();
            def_store.upsert(&definition_record(&def_id)).unwrap();
        }
        state.mstore.set_registry(std::sync::Arc::new(registry));
        state.mstore.set_def_registry(std::sync::Arc::new(def_store));

        let (engine, output_rx) = WshRpcEngine::new();
        register(&engine, &state);
        (state, engine, output_rx, registry_dir, def_dir)
    }

    async fn dispatch_list_recent_sessions(
        engine: &std::sync::Arc<WshRpcEngine>,
        output_rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
    ) -> RpcMessage {
        engine.handle_message(RpcMessage {
            command: COMMAND_LIST_RECENT_SESSIONS.to_string(),
            reqid: "req-1".to_string(),
            data: Some(serde_json::json!({ "limit": 20 })),
            ..Default::default()
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), output_rx.recv())
            .await
            .unwrap()
            .unwrap()
    }

    /// Retro
    /// `docs/retro/retro-my-agents-fresh-channel-regression-2026-07-27.md`
    /// §9 rec 5: this exact path (fresh channel + pre-populated global
    /// registry) had broken three separate times with no regression test
    /// covering it. Directly exercises the RPC end-to-end against a real
    /// `Registry`/`DefinitionStore` on disk — the same code path a fresh
    /// `task package` channel hits on its very first "My Agents" fetch.
    #[tokio::test]
    async fn cross_channel_registry_agents_populate_a_fresh_channels_my_agents_list() {
        let (_state, engine, mut output_rx, _reg_dir, _def_dir) =
            setup_with_n_cross_channel_agents(3);

        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;

        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(
            result.rows.len(),
            3,
            "all 3 cross-channel registry agents must appear on a channel that never created any of them locally"
        );
        assert!(
            result.degraded.is_empty(),
            "a fully healthy call must report no degraded sources, got: {:?}",
            result.degraded
        );
    }

    /// Retro §4/§9 rec 1 — the core hardening this test locks in. Before
    /// this fix, `identity_list()` failing on one unparseable `db_accounts`
    /// row (PR #2296's exact incident: an oauth secret_ref serde-tag
    /// mismatch) `?`-aborted the ENTIRE `listrecentsessions` response,
    /// zeroing "My Agents" even though the registry/definitions themselves
    /// were completely healthy. Reproduces that exact malformed-row shape
    /// directly via raw SQL (bypassing `identity_upsert`, which would
    /// never produce an invalid `secret_ref`) and asserts the response
    /// still contains every agent — only the identity-name enrichment
    /// degrades, not the whole list.
    #[tokio::test]
    async fn a_malformed_identity_account_row_is_tolerated_without_degrading_the_accounts_source() {
        let (state, engine, mut output_rx, _reg_dir, _def_dir) =
            setup_with_n_cross_channel_agents(2);

        // Same malformed shape as the real PR #2296 incident: a
        // `secret_ref.backend` tag no `SecretRef` variant matches.
        //
        // Updated for ANALYSIS_ARMORY_STASH_CREDENTIAL_VISIBILITY_GAP_2026_08_04:
        // `identity_list` itself now skips a malformed row (with a warning)
        // instead of erroring the whole call (backend/storage/identities.rs),
        // so this scenario no longer reaches the `unwrap_or_else` degrade
        // path below at all — the "accounts" source is no longer degraded
        // by a single bad row, it's just quietly correct. The `degraded`
        // push at that call site still exists and is still meaningful for a
        // genuine `identity_list` error (e.g. a real DB failure); it's
        // simply no longer reachable via this specific fixture.
        {
            let conn = state.mstore.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO db_accounts
                    (id, name, provider, kind, display_name, secret_ref, context, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    "acct-broken",
                    "broken-account",
                    "claude",
                    "oauth",
                    "",
                    r#"{"backend":"totally_bogus_backend"}"#,
                    "{}",
                    "unknown",
                    0i64,
                    0i64,
                ],
            )
            .unwrap();
        }

        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;

        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(
            result.rows.len(),
            2,
            "a single malformed db_accounts row must not zero out the whole \
             My Agents list"
        );
        // The improved behavior: identity_list's own per-row tolerance means
        // this is no longer a "source failure" at all — nothing degraded,
        // the malformed row was just silently excluded. Contrast with the
        // OLD assertion this test used to make (`degraded` contains
        // "accounts") — that was evidence of a real, if survivable, failure;
        // this is evidence there wasn't one. The `degraded.push("accounts")`
        // call site (above, in the handler) is still meaningful for a
        // genuine `identity_list` error — just no longer reachable via this
        // fixture.
        assert!(
            !result.degraded.contains(&"accounts".to_string()),
            "a single malformed row must not degrade the accounts source at all \
             (identity_list now tolerates it internally), got: {:?}",
            result.degraded
        );
    }

    /// docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md
    /// §4: when `agent_identity_list_all()` itself fails, no row can say whether
    /// its agent is bound. A failure must not read like a healthy "No auth", so
    /// the account line is left empty (SPEC_MY_AGENTS_TILES_AUTH_AND_HISTORY_2026_10_03.md
    /// §5.2 R5). Forces the failure by dropping `db_agent_identity_links` outright
    /// and asserts every row has an empty account line, with `degraded`
    /// reporting "identity_links".
    #[tokio::test]
    async fn identity_links_source_failure_leaves_the_account_line_empty_not_no_auth() {
        let (state, engine, mut output_rx, _reg_dir, _def_dir) =
            setup_with_n_cross_channel_agents(2);

        {
            let conn = state.mstore.conn().lock().unwrap();
            conn.execute("DROP TABLE db_agent_identity_links", []).unwrap();
        }

        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;

        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(result.rows.len(), 2, "the identity_links failure must not zero out the whole list");
        assert!(
            result.degraded.contains(&"identity_links".to_string()),
            "got: {:?}",
            result.degraded
        );
        for row in &result.rows {
            assert_eq!(
                row.identity_name, "",
                "a source FAILURE must claim nothing: an unbound agent reads \"No auth\" and a \
                 backend outage must not look identical to that"
            );
        }
    }

    /// A failed ACCOUNTS lookup (the global mirror unreadable too) must not turn a
    /// bound agent into "No auth": its account may be perfectly good. The linked
    /// agent's line is left empty; an agent with no link is still plainly unbound.
    #[tokio::test]
    async fn a_failed_accounts_lookup_leaves_a_linked_agents_line_empty_not_no_auth() {
        let (state, engine, mut output_rx, _reg_dir, _def_dir) = setup_with_n_cross_channel_agents(2);
        {
            let conn = state.identity_store.conn().lock().unwrap();
            conn.execute_batch(
                "PRAGMA foreign_keys=OFF; INSERT INTO db_agent_identity_links (agent_id, account_id, provider)                  VALUES ('def-0', 'acct-that-is-fine', 'claude');",
            )
            .unwrap();
        }
        {
            let conn = state.id_store.conn().lock().unwrap();
            conn.execute_batch("PRAGMA foreign_keys=OFF; DROP TABLE db_accounts;").unwrap();
        }
        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;
        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult = serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert!(result.degraded.contains(&"accounts".to_string()), "got: {:?}", result.degraded);
        let line = |def: &str| result.rows.iter().find(|r| r.definition_id == def).unwrap().identity_name.clone();
        assert_eq!(line("def-0"), "", "a link we cannot resolve because the lookup failed is unknown, not unbound");
        assert_eq!(line("def-1"), "No auth", "an agent with no link is unbound whatever the accounts lookup did");
    }

    /// A healthy list: agents with no account link read "No auth", and none of the
    /// retired labels appears anywhere.
    #[tokio::test]
    async fn an_agent_with_no_account_link_says_no_auth_and_the_old_labels_are_gone() {
        let (_state, engine, mut output_rx, _reg_dir, _def_dir) = setup_with_n_cross_channel_agents(2);
        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;
        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult = serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(result.rows.len(), 2);
        for row in &result.rows {
            assert_eq!(row.identity_name, "No auth");
            for old in ["(missing account)", "(ambient creds)", "(unknown account)"] {
                assert_ne!(row.identity_name, old);
            }
        }
    }

    /// Minimal local (not cross-channel registry) `AgentDefinition` —
    /// mirrors `storage/agents/mod.rs`'s own private `test_agent_def` test fixture
    /// shape, duplicated here rather than widening that helper's
    /// visibility for one caller.
    fn local_agent_def(id: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            slug: id.to_string(),
            name: format!("Agent {id}"),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1,
            agent_type: "standalone".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
            seat: None,
            conversation_visibility: "private".to_string(),
        }
    }

    fn local_agent_instance(id: &str, definition_id: &str, block_id: &str) -> AgentInstance {
        AgentInstance {
            id: id.to_string(),
            definition_id: definition_id.to_string(),
            parent_instance_id: String::new(),
            block_id: block_id.to_string(),
            session_id: String::new(),
            status: "available".to_string(),
            github_context: String::new(),
            started_at: 1,
            ended_at: 0,
            created_at: 1,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: id.to_string(),
            working_directory: String::new(),
            display_hidden: false,
        }
    }

    /// docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md
    /// §5a: a row with no structured `output.state.json` snapshot but a
    /// PERSISTED `db_agent_activity_summaries` row for its definition must
    /// surface that summary as `preview`, still reporting `has_snapshot:
    /// false` (accurate — there genuinely is no structured snapshot; the
    /// summary is a substitute, not a claim that one exists).
    #[tokio::test]
    async fn snapshot_less_row_uses_a_persisted_definition_summary_as_preview() {
        let state = test_state();
        let mut def = local_agent_def("def-local-1");
        state.mstore.agent_def_insert(&mut def).unwrap();
        state
            .mstore
            .instance_create(&local_agent_instance("inst-local-1", "def-local-1", "block-local-1"))
            .unwrap();
        state
            .mstore
            .agent_activity_summary_set("def-local-1", "Fixed the login race condition", 1000)
            .unwrap();

        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);
        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;

        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(result.rows.len(), 1);
        let row = &result.rows[0];
        assert!(!row.has_snapshot, "no output.state.json was ever written for this instance");
        assert_eq!(
            row.preview, "Fixed the login race condition",
            "persisted definition summary must surface as the row's preview \
             when there's no real snapshot to preview instead"
        );
    }

    /// Counterpart to the above: when NOTHING is persisted yet for the
    /// definition, the row's `preview` stays empty and the frontend shows its
    /// own empty-state text ("No conversations yet").
    /// Generation itself (a real Haiku CLI round-trip) is fire-and-forget
    /// and not asserted here — this only locks in that a missing summary
    /// never surfaces as a fabricated non-empty preview.
    #[tokio::test]
    async fn snapshot_less_row_with_no_persisted_summary_leaves_preview_empty() {
        let state = test_state();
        let mut def = local_agent_def("def-local-2");
        state.mstore.agent_def_insert(&mut def).unwrap();
        state
            .mstore
            .instance_create(&local_agent_instance("inst-local-2", "def-local-2", "block-local-2"))
            .unwrap();

        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);
        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;

        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(result.rows.len(), 1);
        let row = &result.rows[0];
        assert!(!row.has_snapshot);
        assert_eq!(row.preview, "", "no summary persisted yet — preview must stay empty, not fabricated");
    }

    /// Put an `output` file in the (test) global transcript store for `def_id`.
    fn install_transcript(store: &std::sync::Arc<crate::backend::storage::filestore::FileStore>, def_id: &str, ndjson: &str) {
        let zone = format!("agent:{def_id}:current");
        store
            .make_file(&zone, "output", Default::default(), crate::backend::storage::filestore::FileOpts::default())
            .unwrap();
        store.write_file(&zone, "output", ndjson.as_bytes()).unwrap();
    }

    fn user_line(text: &str) -> String {
        serde_json::json!({"type":"user","message":{"role":"user","content":text}}).to_string()
    }

    /// Clears the thread's test transcript store when a test ends, pass or fail.
    struct TranscriptStoreGuard;
    impl Drop for TranscriptStoreGuard {
        fn drop(&mut self) {
            crate::backend::agent_session::history_summary::set_test_store(None);
        }
    }

    /// The preview, last-active time and "has history" flag come from the agent's
    /// global transcript, so a registry-only row (no local instance, no block id)
    /// with a transcript shows what it was last asked.
    #[tokio::test]
    async fn a_row_with_no_block_takes_its_preview_from_the_global_transcript() {
        let (_state, engine, mut output_rx, _reg_dir, _def_dir) = setup_with_n_cross_channel_agents(2);
        let store = std::sync::Arc::new(crate::backend::storage::filestore::FileStore::open_in_memory().unwrap());
        install_transcript(&store, "def-0", &format!("{}\n{}", user_line("# Session Context\nboot"), user_line("ship the tiles")));
        crate::backend::agent_session::history_summary::set_test_store(Some(store));
        let _guard = TranscriptStoreGuard;

        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;
        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult = serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        let with = result.rows.iter().find(|r| r.definition_id == "def-0").unwrap();
        assert!(with.block_id_hint.is_empty(), "the row has no local block");
        assert!(with.has_snapshot);
        assert!(!with.snapshot_check_failed);
        assert_eq!(with.preview, "ship the tiles");
        assert!(with.last_active_at > 1, "last-active comes from the transcript, got {}", with.last_active_at);
        let without = result.rows.iter().find(|r| r.definition_id == "def-1").unwrap();
        assert!(!without.has_snapshot, "an agent with no transcript has no history");
        assert!(!without.snapshot_check_failed, "no transcript is a state, not a failed read");
        assert_eq!(without.preview, "");
    }

    /// A transcript that holds no user prompt is still history: the row says so
    /// (has_snapshot) rather than claiming the agent never ran.
    #[tokio::test]
    async fn a_transcript_without_a_prompt_still_counts_as_history() {
        let (_state, engine, mut output_rx, _reg_dir, _def_dir) = setup_with_n_cross_channel_agents(1);
        let store = std::sync::Arc::new(crate::backend::storage::filestore::FileStore::open_in_memory().unwrap());
        install_transcript(&store, "def-0", r#"{"type":"stream_event","event":{}}"#);
        crate::backend::agent_session::history_summary::set_test_store(Some(store));
        let _guard = TranscriptStoreGuard;

        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;
        let result: ListRecentSessionsResult = serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert!(result.rows[0].has_snapshot);
        assert_eq!(result.rows[0].preview, "");
    }

    /// Consolidation Phase 3b (reagent P1 on PR #3080): the row's display
    /// fields must come from the AGENT's own row, never from the template it
    /// was launched off. The template is consulted for exactly one thing —
    /// the legacy-shaped identity-link fallback — and an agent whose
    /// provider/name/type has since drifted from its template must report its
    /// own current values, as it did before consolidation.
    #[tokio::test]
    async fn a_launched_agents_row_reports_its_own_config_not_its_templates() {
        let state = test_state();
        let mut tmpl = local_agent_def("tmpl-drift");
        tmpl.is_seeded = 1;
        tmpl.name = "Template".to_string();
        tmpl.provider = "claude".to_string();
        tmpl.agent_type = "standalone".to_string();
        state.mstore.agent_def_insert(&mut tmpl).unwrap();

        let mut own = local_agent_def("agent-drift");
        own.parent_id = "tmpl-drift".to_string();
        state.mstore.agent_def_insert(&mut own).unwrap();
        state
            .mstore
            .instance_create(&local_agent_instance("launch-1", "agent-drift", "block-drift"))
            .unwrap();

        // The agent drifts from the template it was cloned from.
        own.name = "Renamed by the user".to_string();
        own.provider = "codex".to_string();
        own.agent_type = "orchestrator".to_string();
        state.mstore.agent_def_update(&mut own).unwrap();

        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);
        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;

        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(result.rows.len(), 1);
        let row = &result.rows[0];
        assert_eq!(row.definition_name, "Renamed by the user");
        assert_eq!(row.provider, "codex", "the template's 'claude' must not shadow the agent's own provider");
        assert_eq!(row.agent_type, "orchestrator");
    }

    /// docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md
    /// §5, bullet 1: `filestore.stat()`'s `Err` (I/O error, lock
    /// contention, DB read failure) used to collapse into the exact same
    /// branch as `Ok(None)` (genuinely no snapshot) — a transient storage
    /// error on a row that DOES have real history rendered identically to
    /// "never had one," with no log line. Forces the failure
    /// deterministically the same way the identity_links test above does
    /// (`DROP TABLE`), rather than relying on real I/O flakiness.
    #[tokio::test]
    async fn filestore_stat_error_is_distinguished_from_genuinely_no_snapshot() {
        let state = test_state();
        let mut def = local_agent_def("def-local-3");
        state.mstore.agent_def_insert(&mut def).unwrap();
        state
            .mstore
            .instance_create(&local_agent_instance("inst-local-3", "def-local-3", "block-local-3"))
            .unwrap();

        {
            let conn = state.filestore.conn().lock().unwrap();
            conn.execute("DROP TABLE db_wave_file", []).unwrap();
        }

        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);
        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;

        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        assert_eq!(result.rows.len(), 1, "a stat() failure on one row must not zero out the whole list");
        let row = &result.rows[0];
        assert!(!row.has_snapshot, "a failed stat() is not evidence a snapshot exists");
        assert!(
            row.snapshot_check_failed,
            "a stat() I/O error must be distinguishable from a genuine Ok(None) — \
             otherwise a transient failure looks identical to \"never had history\""
        );
        assert!(
            result.degraded.contains(&"snapshot_stat".to_string()),
            "got: {:?}",
            result.degraded
        );
    }

    /// Counterpart: a genuine `Ok(None)` (the block really never wrote a
    /// snapshot) must NOT set `snapshot_check_failed` — only an actual
    /// `Err` does. Without this, the fix above could be made to pass by
    /// setting the flag unconditionally whenever `has_snapshot` is false,
    /// which would just move the ambiguity rather than remove it.
    #[tokio::test]
    async fn genuinely_missing_snapshot_does_not_set_the_check_failed_flag() {
        let state = test_state();
        let mut def = local_agent_def("def-local-4");
        state.mstore.agent_def_insert(&mut def).unwrap();
        state
            .mstore
            .instance_create(&local_agent_instance("inst-local-4", "def-local-4", "block-local-4"))
            .unwrap();

        let (engine, mut output_rx) = WshRpcEngine::new();
        register(&engine, &state);
        let resp = dispatch_list_recent_sessions(&engine, &mut output_rx).await;

        assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
        let result: ListRecentSessionsResult =
            serde_json::from_value(resp.data.expect("expected result data")).unwrap();
        let row = &result.rows[0];
        assert!(!row.has_snapshot);
        assert!(
            !row.snapshot_check_failed,
            "a genuine Ok(None) — the block simply never wrote a snapshot — is not a check failure"
        );
    }

    /// reagent P1, PR #2786: without a dedup gate, a definition that can
    /// never be summarized (no cmd meta, no raw output, CLI failure) gets
    /// re-spawned on every `listrecentsessions` poll forever —
    /// `ambient::gateway().admit()` alone only coalesces CONCURRENT
    /// triggers, not sequential ones, since its guard drops (freeing the
    /// slot) the moment one call finishes. This tests
    /// `definition_summary_attempted()`'s claim set directly rather than
    /// the RPC dispatch (which can't observe whether a background
    /// `tokio::spawn` happened without mocking the Haiku CLI) — a unique
    /// id avoids cross-test interference with the process-wide static set.
    #[test]
    fn definition_summary_attempted_claims_a_definition_id_at_most_once() {
        let id = "def-dedup-test-unique-9f3a1b".to_string();
        assert!(
            definition_summary_attempted().lock().unwrap().insert(id.clone()),
            "first claim for a fresh id must succeed"
        );
        assert!(
            !definition_summary_attempted().lock().unwrap().insert(id.clone()),
            "second claim for the same id must be rejected — this is the fix for \
             reagent's unbounded-retry finding on PR #2786"
        );
    }
}
