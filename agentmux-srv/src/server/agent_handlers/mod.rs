// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

mod core;
mod skills;
mod history;
mod template;
mod identity;
mod instance;
mod session;
mod memory;
mod input;

use std::sync::Arc;


use crate::backend::rpc::engine::WshRpcEngine;

use super::AppState;

pub use input::register_agent_input_handlers;

pub fn register_agent_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    core::register(engine, state);
    skills::register(engine, state);
    history::register(engine, state);
    template::register(engine, state);
    identity::register(engine, state);
    instance::register(engine, state);
    session::register(engine, state);
    memory::register(engine, state);
}

/// Read the per-block `output.state.json` snapshot from filestore and
/// extract a `(preview, node_count)` pair for the AgentPicker's
/// "Recent sessions" list.
///
/// The snapshot shape is owned by the frontend (see
/// `frontend/app/view/agent/agent-view.tsx::writeSnapshotNow`):
/// `{ schemaVersion, savedAt, highWaterMark, historyOffset, nodes: [DocumentNode...] }`.
/// We only touch two fields:
/// - `nodes.length` → `node_count`.
/// - The first node with `type === "user_message"`, `message` field →
///   `preview` (trimmed, newlines collapsed, max 240 chars).
///
/// On any error (snapshot missing, malformed JSON, no user message),
/// returns `("", 0)`. Callers treat that the same as "no preview".
fn read_session_preview(
    filestore: &crate::backend::storage::filestore::FileStore,
    block_id: &str,
) -> (String, usize) {
    let bytes = match filestore.read_file(block_id, "output.state.json") {
        Ok(Some(b)) => b,
        _ => return (String::new(), 0),
    };
    // Cap the parse budget — a misbehaving / corrupted snapshot
    // shouldn't be able to stall this handler. 4MiB is well above the
    // typical conversation snapshot (Maks's was ~750KiB for 169 nodes)
    // but bounded enough to fail fast on garbage.
    if bytes.len() > 4 * 1024 * 1024 {
        tracing::warn!(
            block_id = %block_id,
            size = bytes.len(),
            "listrecentsessions: snapshot too large; skipping preview"
        );
        return (String::new(), 0);
    }
    let json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return (String::new(), 0),
    };
    let nodes = match json.get("nodes").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return (String::new(), 0),
    };
    let node_count = nodes.len();
    // First user_message wins. Skip the bootstrap "Session Context"
    // prompt when present — it's always the first node and is system
    // boilerplate the user didn't type; if a subsequent user_message
    // exists, that's the more useful preview. Heuristic: if the first
    // user message starts with "# Session Context", scan for the next.
    let mut preview = String::new();
    for node in nodes {
        let ty = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if ty != "user_message" {
            continue;
        }
        let msg = node
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if msg.is_empty() {
            continue;
        }
        if preview.is_empty() && msg.starts_with("# Session Context") {
            // Stash as fallback in case there's no later user_message.
            preview = collapse_preview(msg);
            continue;
        }
        preview = collapse_preview(msg);
        break;
    }
    (preview, node_count)
}

/// Collapse newlines + extra whitespace, cap at 240 chars. Output is
/// safe to render inline in a single-line preview row.
fn collapse_preview(s: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut buf = String::with_capacity(s.len().min(MAX_CHARS + 4));
    let mut prev_space = false;
    for ch in s.chars() {
        if buf.chars().count() >= MAX_CHARS {
            buf.push('\u{2026}'); // "…"
            return buf;
        }
        if ch.is_whitespace() {
            if !prev_space && !buf.is_empty() {
                buf.push(' ');
                prev_space = true;
            }
        } else {
            buf.push(ch);
            prev_space = false;
        }
    }
    buf
}

#[cfg(test)]
mod recent_sessions_tests {
    use super::*;
    use crate::backend::rpc_types::{ListRecentSessionsResult, COMMAND_LIST_RECENT_SESSIONS};
    use crate::backend::storage::filestore::FileStore;

    fn fresh_filestore() -> std::sync::Arc<FileStore> {
        std::sync::Arc::new(FileStore::open_in_memory().unwrap())
    }

    fn write_snapshot(fs: &FileStore, block_id: &str, body: &str) {
        // make_file then write_file mirrors the production
        // BlockfileWriteState handler path.
        let meta: crate::backend::storage::filestore::FileMeta =
            std::collections::HashMap::new();
        let opts = crate::backend::storage::filestore::FileOpts::default();
        fs.make_file(block_id, "output.state.json", meta, opts)
            .expect("make_file");
        fs.write_file(block_id, "output.state.json", body.as_bytes())
            .expect("write_file");
    }

    #[test]
    fn collapse_preview_strips_newlines_and_caps_length() {
        let s = "hello\n\nworld\n  next   line";
        assert_eq!(collapse_preview(s), "hello world next line");
        let long: String = "a".repeat(500);
        let out = collapse_preview(&long);
        // 240 chars + ellipsis.
        assert!(out.ends_with('\u{2026}'));
        assert!(out.chars().count() <= 241);
    }

    #[test]
    fn read_session_preview_missing_returns_zero() {
        let fs = fresh_filestore();
        let (preview, count) = read_session_preview(&fs, "no-such-block");
        assert_eq!(preview, "");
        assert_eq!(count, 0);
    }

    #[test]
    fn read_session_preview_extracts_first_user_message_skipping_context() {
        let fs = fresh_filestore();
        // Two user messages: first is the boilerplate Session Context;
        // second is the user's real prompt. Preview should be the real one.
        let snapshot = serde_json::json!({
            "schemaVersion": 1,
            "savedAt": "2026-05-23T08:00:00Z",
            "highWaterMark": 169,
            "historyOffset": 0,
            "nodes": [
                {
                    "type": "user_message",
                    "id": "u0",
                    "timestamp": 0,
                    "collapsed": false,
                    "summary": "👤 User Message",
                    "message": "# Session Context\nIdentity: Claude\n## Description\nStartup boilerplate"
                },
                { "type": "markdown", "id": "m0", "content": "ack" },
                {
                    "type": "user_message",
                    "id": "u1",
                    "timestamp": 100,
                    "collapsed": false,
                    "summary": "👤 User Message",
                    "message": "check the agentmuxai/agentmux history, get the latest code"
                }
            ]
        });
        write_snapshot(&fs, "blk-1", &snapshot.to_string());
        let (preview, count) = read_session_preview(&fs, "blk-1");
        assert_eq!(count, 3);
        assert!(preview.starts_with("check the agentmuxai/agentmux"));
    }

    #[test]
    fn read_session_preview_falls_back_to_session_context_when_only_one() {
        let fs = fresh_filestore();
        let snapshot = serde_json::json!({
            "schemaVersion": 1,
            "nodes": [
                {
                    "type": "user_message",
                    "id": "u0",
                    "message": "# Session Context\nIdentity: Claude\nStartup boilerplate"
                }
            ]
        });
        write_snapshot(&fs, "blk-2", &snapshot.to_string());
        let (preview, count) = read_session_preview(&fs, "blk-2");
        assert_eq!(count, 1);
        // Newlines collapsed; starts with the boilerplate marker.
        assert!(preview.starts_with("# Session Context"));
    }

    #[test]
    fn read_session_preview_handles_malformed_json() {
        let fs = fresh_filestore();
        write_snapshot(&fs, "blk-3", "not valid json {");
        let (preview, count) = read_session_preview(&fs, "blk-3");
        assert_eq!(preview, "");
        assert_eq!(count, 0);
    }

    #[test]
    fn read_session_preview_handles_no_user_messages() {
        let fs = fresh_filestore();
        let snapshot = serde_json::json!({
            "schemaVersion": 1,
            "nodes": [
                { "type": "markdown", "id": "m0", "content": "system note" }
            ]
        });
        write_snapshot(&fs, "blk-4", &snapshot.to_string());
        let (preview, count) = read_session_preview(&fs, "blk-4");
        assert_eq!(preview, "");
        assert_eq!(count, 1);
    }

    // ── Integration test: full listrecentsessions handler ────────────
    //
    // Spins up the same engine + state shape as the production
    // websocket path so the handler runs end-to-end against an
    // in-memory wstore + filestore. Asserts the row shape, the
    // identity filter, the snapshot-first sort, the preview extraction,
    // and the cross-version "no snapshot" fallback. This is the
    // backend correctness gate for the AgentPicker's Recent Sessions
    // surface (cascade follow-up 2026-05-23).
    use crate::backend::storage::store::{
        AgentDefinition, AgentInstance, IdentityAccount, InstanceStatus, Memory, SecretRef, Store,
    };
    use crate::backend::rpc::engine::WshRpcEngine;
    use crate::server::AppState;
    use std::sync::Arc;

    /// Drive a single RPC round-trip against the in-memory engine,
    /// asserting success + deserializing the JSON payload into `T`.
    async fn call_rpc<T: serde::de::DeserializeOwned>(
        engine: &Arc<WshRpcEngine>,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::backend::rpc_types::RpcMessage>,
        command: &str,
        data: serde_json::Value,
    ) -> T {
        let req_id = format!("test-{}", uuid::Uuid::new_v4());
        let msg = crate::backend::rpc_types::RpcMessage {
            command: command.to_string(),
            reqid: req_id.clone(),
            data: Some(data),
            ..Default::default()
        };
        engine.handle_message(msg);
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("handler timed out")
            .expect("output channel closed");
        assert_eq!(resp.resid, req_id, "unexpected response id");
        assert!(resp.error.is_empty(), "handler returned error: {}", resp.error);
        let payload = resp.data.unwrap_or(serde_json::Value::Null);
        serde_json::from_value(payload).expect("response deserialize")
    }

    fn build_state_with_seed() -> (
        AppState,
        Arc<WshRpcEngine>,
        tokio::sync::mpsc::UnboundedReceiver<crate::backend::rpc_types::RpcMessage>,
    ) {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let event_bus = Arc::new(crate::backend::eventbus::EventBus::new());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let reactive_handler = crate::backend::reactive::get_global_handler();
        let poller = Arc::new(crate::backend::reactive::Poller::new(
            crate::backend::reactive::PollerConfig {
                muxbus_url: None,
                muxbus_token: None,
                poll_interval_secs: 30,
            },
            reactive_handler,
        ));
        crate::backend::wcore::ensure_initial_data(&wstore).unwrap();
        let config_watcher = Arc::new(crate::backend::wconfig::ConfigWatcher::new());
        let process_tracker = Arc::new(
            crate::backend::process_tracker::registry::AgentProcessRegistry::new(Some(broker.clone())),
        );
        let process_broker = Arc::new(crate::broker::ProcessBroker::new(Some(broker.clone())));
        let fs_watch_pool = crate::backend::fs_watch::FsWatchPool::new();
        let state = AppState {
            auth_key: "test".to_string(),
            lan_key: "test-lan".to_string(),
            boot_id: std::sync::Arc::from("test-boot"),
            version: "test".to_string(),
            app_path: String::new(),
            wstore: wstore.clone(),
            shared_store: None,
            id_store: wstore.clone(),
            filestore: filestore.clone(),
            global_transcript_store: None,
            event_bus: event_bus.clone(),
            broker: broker.clone(),
            reactive_handler,
            poller,
            config_watcher,
            messagebus: Arc::new(crate::backend::messagebus::MessageBus::new()),
            http_client: reqwest::Client::new(),
            local_web_url: String::new(),
            subagent_watcher: Arc::new(crate::backend::subagent_watcher::SubagentWatcher::new(event_bus.clone(), wstore.clone())),
            history_service: Arc::new(crate::backend::history::HistoryService::new()),
            lan_discovery: Arc::new(crate::backend::lan_discovery::LanDiscoveryController::new(
                "test-instance".to_string(),
                "test-host".to_string(),
                "0.28.20".to_string(),
                0,
                event_bus.clone(),
                String::new(),
            )),
            lsp_supervisor: Arc::new(crate::backend::lsp::LspSupervisor::new(event_bus.clone())),
            process_tracker,
            process_broker,
            dock_snapshots: Arc::new(crate::backend::dock_snapshot::DockSnapshotCache::new()),
            srv_state: Arc::new(tokio::sync::Mutex::new(crate::state::State::default())),
            srv_events_tx: tokio::sync::broadcast::channel::<agentmux_common::ipc::Event>(64).0,
            saga_id_alloc: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            saga_log: Arc::new(crate::sagas::log::SagaLog::open_in_memory().unwrap()),
            auth_session_manager: Arc::new(crate::identity::auth_session::AuthSessionManager::new()),
            install_sessions: crate::server::install_handlers::InstallSessionRegistry::new(),
            container_manager: Arc::new(crate::backend::container::ContainerRuntimeHandle::disabled()),
            shell_sessions: crate::backend::shell_node::ShellSessionRegistry::new(),
            cron_scheduler: crate::backend::cron::CronScheduler::new(
                None,
                reqwest::Client::new(),
                String::new(),
                "test".to_string(),
                broker.clone(),
            ),
            editor_file_watcher: crate::backend::editor_file_watcher::EditorFileWatcher::new(
                fs_watch_pool.clone(),
                broker.clone(),
            ),
            media_file_watcher: crate::backend::media_file_watcher::MediaFileWatcher::new(
                fs_watch_pool.clone(),
                broker.clone(),
            ),
            fs_watch_pool: fs_watch_pool.clone(),
        };

        // Seed: 1 SEEDED definition (template), 1 account + direct
        // identity link, 1 memory bundle. Phase 3b note: seeded as a
        // template so that
        // each instance projection in `db_agents` lands on its own row
        // (`is_template = 0`, `id = inst.id`, `parent_template_id =
        // def.id`) rather than folding into the def-projection and
        // clobbering its name. The handler resolves `definition_name`
        // via `defs.iter().find(|d| d.id == inst.definition_id)`, which
        // hits the template row and returns "Claude Code". Under the
        // pre-Phase 3b reader, def name was always preserved because
        // `agent_def_list` queried `db_agent_definitions` directly;
        // db_agents fold semantics require the seed shape to avoid
        // the collision.
        let def = AgentDefinition {
            id: "def-claude".to_string(),
            slug: "claude-code".to_string(),
            name: "Claude Code".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        };
        let mut def_mut = def.clone();
        wstore.agent_def_insert(&mut def_mut).unwrap();
        // Identity display name resolves via the direct agent<->account
        // link (db_agent_identity_links/db_accounts) now, not a bundle —
        // see agent_handlers::session's listrecentsessions.
        let account = IdentityAccount {
            id: "acct-work".to_string(),
            name: "Work".to_string(),
            provider: "github".to_string(),
            kind: "pat".to_string(),
            display_name: String::new(),
            secret_ref: SecretRef::Env { env_var: "GITHUB_TOKEN".to_string() },
            context: serde_json::json!({}),
            status: "unknown".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        wstore.identity_upsert(&account).unwrap();
        wstore
            .agent_identity_link("def-claude", "acct-work", "github")
            .unwrap();
        let memory = Memory {
            id: "mem-notes".to_string(),
            name: "Notes".to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        };
        wstore.bundle_memory_upsert(&memory).unwrap();

        // 3 instances:
        //   - blk-recent: has snapshot, more recent activity
        //   - blk-older:  has snapshot, older activity
        //   - blk-none:   no snapshot at all (legacy / pre-persistence row)
        // All three share the same identity_id value so the filter test
        // can also exercise it without re-seeding.
        for (id, block, started) in [
            ("inst-recent", "blk-recent", 1_700_000_100_000_i64),
            ("inst-older", "blk-older", 1_700_000_000_000_i64),
            ("inst-none", "blk-none", 1_700_000_050_000_i64),
        ] {
            let inst = AgentInstance {
                id: id.to_string(),
                definition_id: "def-claude".to_string(),
                parent_instance_id: String::new(),
                block_id: block.to_string(),
                session_id: String::new(),
                status: InstanceStatus::Running.as_str().to_string(),
                github_context: String::new(),
                started_at: started,
                ended_at: 0,
                created_at: started,
                identity_id: "id-work".to_string(),
                memory_id: "mem-notes".to_string(),
                instance_name: format!("name-{id}"),
                working_directory: format!("/tmp/{id}"),
                display_hidden: false,
            };
            wstore.instance_create(&inst).unwrap();
        }

        // Snapshots for the two with snapshots. Write the OLDER one
        // first so its filestore-stamped modts is strictly less than the
        // recent one — the handler sorts snapshot-bearing rows by modts
        // desc, so writing blk-older second would invert the assertions.
        // (Pre-Phase 3b this ordering was fragile because the dual-write
        // chain ran fewer SQL statements between successive inserts, so
        // adjacent writes landed in the same millisecond and the stable
        // sort preserved instance_list_named's started_at order; now the
        // additional db_agents UPDATE per instance widens the gap and
        // distinct modts dominate the stable sort.)
        let snap_older = serde_json::json!({
            "schemaVersion": 1,
            "nodes": [
                {"type": "user_message", "id": "u0",
                 "message": "earlier conversation"}
            ]
        });
        write_snapshot(&filestore, "blk-older", &snap_older.to_string());
        let snap_recent = serde_json::json!({
            "schemaVersion": 1,
            "nodes": [
                {"type": "user_message", "id": "u0",
                 "message": "# Session Context\nboilerplate"},
                {"type": "markdown", "id": "m0", "content": "ack"},
                {"type": "user_message", "id": "u1",
                 "message": "fix the live-feed hover delay"}
            ]
        });
        write_snapshot(&filestore, "blk-recent", &snap_recent.to_string());

        let (engine, rx) = WshRpcEngine::new();
        super::register_agent_handlers(&engine, &state);
        (state, engine, rx)
    }

    #[tokio::test]
    async fn handler_returns_sessions_with_previews_sorted_by_snapshot_first() {
        let (_state, engine, mut rx) = build_state_with_seed();
        let result: ListRecentSessionsResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_LIST_RECENT_SESSIONS,
            serde_json::json!({}),
        )
        .await;
        let rows = result.rows;
        assert!(result.degraded.is_empty(), "healthy seed must report no degraded sources");
        assert_eq!(rows.len(), 3, "all three sessions surfaced");

        // Sort: snapshot-bearing rows first (recent then older), then
        // the no-snapshot row at the tail.
        assert_eq!(rows[0].instance_id, "inst-recent");
        assert!(rows[0].has_snapshot);
        assert_eq!(rows[0].node_count, 3);
        assert!(
            rows[0].preview.starts_with("fix the live-feed"),
            "preview should be the post-context user message, got {:?}",
            rows[0].preview
        );

        assert_eq!(rows[1].instance_id, "inst-older");
        assert!(rows[1].has_snapshot);
        assert_eq!(rows[1].node_count, 1);
        assert_eq!(rows[1].preview, "earlier conversation");

        assert_eq!(rows[2].instance_id, "inst-none");
        assert!(!rows[2].has_snapshot);
        assert_eq!(rows[2].node_count, 0);
        assert_eq!(rows[2].preview, "");

        // Joins: definition + identity + memory names resolved.
        assert_eq!(rows[0].definition_name, "Claude Code");
        assert_eq!(rows[0].identity_name, "Work");
        assert_eq!(rows[0].memory_name, "Notes");
        assert_eq!(rows[0].block_id_hint, "blk-recent");
    }

    #[tokio::test]
    async fn handler_identity_filter_restricts_rows() {
        let (_state, engine, mut rx) = build_state_with_seed();
        // Filter to a non-existent identity → empty list.
        let result: ListRecentSessionsResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_LIST_RECENT_SESSIONS,
            serde_json::json!({ "identity_id": "no-such-bundle" }),
        )
        .await;
        assert_eq!(result.rows.len(), 0);

        // Filter to the seeded one → all three.
        let result: ListRecentSessionsResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_LIST_RECENT_SESSIONS,
            serde_json::json!({ "identity_id": "id-work" }),
        )
        .await;
        assert_eq!(result.rows.len(), 3);

        // Empty-string identity_id is treated as "no filter" so the
        // frontend can pass `""` without special-casing.
        let result: ListRecentSessionsResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_LIST_RECENT_SESSIONS,
            serde_json::json!({ "identity_id": "" }),
        )
        .await;
        assert_eq!(result.rows.len(), 3);
    }

    #[tokio::test]
    async fn handler_respects_limit() {
        let (_state, engine, mut rx) = build_state_with_seed();
        let result: ListRecentSessionsResult = call_rpc(
            &engine,
            &mut rx,
            COMMAND_LIST_RECENT_SESSIONS,
            serde_json::json!({ "limit": 1 }),
        )
        .await;
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].instance_id, "inst-recent");
    }

    // ---- Two-tier picker Phase 1: create-from-template + listagents filter ----
    //
    // SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md.

    /// Same shape as build_state_with_seed but with a seeded template
    /// and no instances, so the create-from-template path is exercised
    /// against a known-good template row.
    fn build_state_with_template_seed() -> (
        AppState,
        Arc<WshRpcEngine>,
        tokio::sync::mpsc::UnboundedReceiver<crate::backend::rpc_types::RpcMessage>,
    ) {
        let wstore = Arc::new(Store::open_in_memory().unwrap());
        let filestore = Arc::new(FileStore::open_in_memory().unwrap());
        let event_bus = Arc::new(crate::backend::eventbus::EventBus::new());
        let broker = Arc::new(crate::backend::wps::Broker::new());
        let reactive_handler = crate::backend::reactive::get_global_handler();
        let poller = Arc::new(crate::backend::reactive::Poller::new(
            crate::backend::reactive::PollerConfig {
                muxbus_url: None,
                muxbus_token: None,
                poll_interval_secs: 30,
            },
            reactive_handler,
        ));
        crate::backend::wcore::ensure_initial_data(&wstore).unwrap();
        let config_watcher = Arc::new(crate::backend::wconfig::ConfigWatcher::new());
        let process_tracker = Arc::new(
            crate::backend::process_tracker::registry::AgentProcessRegistry::new(Some(broker.clone())),
        );
        let process_broker = Arc::new(crate::broker::ProcessBroker::new(Some(broker.clone())));
        let fs_watch_pool = crate::backend::fs_watch::FsWatchPool::new();
        let state = AppState {
            auth_key: "test".to_string(),
            lan_key: "test-lan".to_string(),
            boot_id: std::sync::Arc::from("test-boot"),
            version: "test".to_string(),
            app_path: String::new(),
            wstore: wstore.clone(),
            shared_store: None,
            id_store: wstore.clone(),
            filestore: filestore.clone(),
            global_transcript_store: None,
            event_bus: event_bus.clone(),
            broker: broker.clone(),
            reactive_handler,
            poller,
            config_watcher,
            messagebus: Arc::new(crate::backend::messagebus::MessageBus::new()),
            http_client: reqwest::Client::new(),
            local_web_url: String::new(),
            subagent_watcher: Arc::new(crate::backend::subagent_watcher::SubagentWatcher::new(event_bus.clone(), wstore.clone())),
            history_service: Arc::new(crate::backend::history::HistoryService::new()),
            lan_discovery: Arc::new(crate::backend::lan_discovery::LanDiscoveryController::new(
                "test-instance".to_string(),
                "test-host".to_string(),
                "0.28.20".to_string(),
                0,
                event_bus.clone(),
                String::new(),
            )),
            lsp_supervisor: Arc::new(crate::backend::lsp::LspSupervisor::new(event_bus.clone())),
            process_tracker,
            process_broker,
            dock_snapshots: Arc::new(crate::backend::dock_snapshot::DockSnapshotCache::new()),
            srv_state: Arc::new(tokio::sync::Mutex::new(crate::state::State::default())),
            srv_events_tx: tokio::sync::broadcast::channel::<agentmux_common::ipc::Event>(64).0,
            saga_id_alloc: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            saga_log: Arc::new(crate::sagas::log::SagaLog::open_in_memory().unwrap()),
            auth_session_manager: Arc::new(crate::identity::auth_session::AuthSessionManager::new()),
            install_sessions: crate::server::install_handlers::InstallSessionRegistry::new(),
            container_manager: Arc::new(crate::backend::container::ContainerRuntimeHandle::disabled()),
            shell_sessions: crate::backend::shell_node::ShellSessionRegistry::new(),
            cron_scheduler: crate::backend::cron::CronScheduler::new(
                None,
                reqwest::Client::new(),
                String::new(),
                "test".to_string(),
                broker.clone(),
            ),
            editor_file_watcher: crate::backend::editor_file_watcher::EditorFileWatcher::new(
                fs_watch_pool.clone(),
                broker.clone(),
            ),
            media_file_watcher: crate::backend::media_file_watcher::MediaFileWatcher::new(
                fs_watch_pool.clone(),
                broker.clone(),
            ),
            fs_watch_pool: fs_watch_pool.clone(),
        };

        // One seeded template + one already-user-owned definition.
        let mut tpl = AgentDefinition {
            id: "tpl-claude".to_string(),
            slug: String::new(),
            name: "Claude Code".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: "Anthropic's coding agent".to_string(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: "--model haiku".to_string(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_000_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 1,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_000_000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        };
        wstore.agent_def_insert(&mut tpl).unwrap();

        let mut user_a = AgentDefinition {
            id: "user-a".to_string(),
            slug: String::new(),
            name: "Maks".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_001_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_001_000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
        };
        wstore.agent_def_insert(&mut user_a).unwrap();

        let (engine, rx) = WshRpcEngine::new();
        super::register_agent_handlers(&engine, &state);
        (state, engine, rx)
    }

    #[tokio::test]
    async fn listagents_no_filter_returns_all() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let agents: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({}),
        )
        .await;
        assert!(agents.iter().any(|a| a.id == "tpl-claude"));
        assert!(agents.iter().any(|a| a.id == "user-a"));
    }

    #[tokio::test]
    async fn listagents_filter_templates_only() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let agents: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({ "is_seeded": 1 }),
        )
        .await;
        assert!(agents.iter().all(|a| a.is_seeded == 1));
        assert!(agents.iter().any(|a| a.id == "tpl-claude"));
        assert!(!agents.iter().any(|a| a.id == "user-a"));
    }

    #[tokio::test]
    async fn listagents_filter_user_owned_only() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let agents: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({ "is_seeded": 0 }),
        )
        .await;
        assert!(agents.iter().all(|a| a.is_seeded == 0));
        assert!(agents.iter().any(|a| a.id == "user-a"));
        assert!(!agents.iter().any(|a| a.id == "tpl-claude"));
    }

    #[tokio::test]
    async fn create_from_template_happy_path_clones_and_returns_id() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let resp: crate::backend::rpc_types::AgentDefCreateFromTemplateResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
            serde_json::json!({
                "template_id": "tpl-claude",
                "name": "Asaf",
                "identity_id": "id-work",
                "memory_id": "mem-notes",
            }),
        )
        .await;
        assert!(!resp.definition_id.is_empty());
        assert_eq!(resp.identity_id, "id-work");
        assert_eq!(resp.memory_id, "mem-notes");

        // The new row is user-owned, carries provider + flags from template.
        let agents: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({}),
        )
        .await;
        let new_def = agents
            .iter()
            .find(|a| a.id == resp.definition_id)
            .expect("new definition should appear in listagents");
        assert_eq!(new_def.is_seeded, 0);
        assert_eq!(new_def.name, "Asaf");
        assert_eq!(new_def.provider, "claude");
        assert_eq!(new_def.provider_flags, "--model haiku");
        assert_eq!(new_def.parent_id, "tpl-claude");
    }

    async fn call_rpc_expect_error(
        engine: &Arc<WshRpcEngine>,
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::backend::rpc_types::RpcMessage>,
        command: &str,
        data: serde_json::Value,
    ) -> String {
        let req_id = format!("test-{}", uuid::Uuid::new_v4());
        let msg = crate::backend::rpc_types::RpcMessage {
            command: command.to_string(),
            reqid: req_id.clone(),
            data: Some(data),
            ..Default::default()
        };
        engine.handle_message(msg);
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("handler timed out")
            .expect("output channel closed");
        assert_eq!(resp.resid, req_id);
        assert!(
            !resp.error.is_empty(),
            "expected error, got success payload: {:?}",
            resp.data
        );
        resp.error
    }

    #[tokio::test]
    async fn create_from_template_rejects_non_template_id() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        // "user-a" is is_seeded=0 — not a template.
        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
            serde_json::json!({
                "template_id": "user-a",
                "name": "another",
            }),
        )
        .await;
        assert!(
            err.contains("not a seeded template"),
            "wrong error: {err}"
        );
    }

    #[tokio::test]
    async fn create_from_template_rejects_unknown_template_id() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
            serde_json::json!({
                "template_id": "no-such-id",
                "name": "x",
            }),
        )
        .await;
        assert!(err.contains("not found"), "wrong error: {err}");
    }

    #[tokio::test]
    async fn create_from_template_rejects_duplicate_user_name() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        // "Maks" already exists as a user-owned agent.
        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
            serde_json::json!({
                "template_id": "tpl-claude",
                "name": "Maks",
            }),
        )
        .await;
        assert!(
            err.contains("already exists"),
            "wrong error: {err}"
        );
    }

    #[tokio::test]
    async fn create_from_template_rejects_empty_name() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_CREATE_FROM_TEMPLATE,
            serde_json::json!({
                "template_id": "tpl-claude",
                "name": "   ",
            }),
        )
        .await;
        assert!(err.contains("non-empty"), "wrong error: {err}");
    }

    // ---- Two-tier picker Phase 2: hide / unhide templates ----
    //
    // SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md Q2 Decision Y.

    #[tokio::test]
    async fn hide_template_then_listagents_excludes_it_by_default() {
        let (_state, engine, mut rx) = build_state_with_template_seed();

        // Before hide: template is in the default listagents result.
        let before: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({}),
        )
        .await;
        assert!(before.iter().any(|a| a.id == "tpl-claude"));

        // Hide the template.
        let resp: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "tpl-claude" }),
        )
        .await;
        assert!(resp.ok);

        // After hide: default listagents no longer surfaces it.
        let after: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({}),
        )
        .await;
        assert!(
            !after.iter().any(|a| a.id == "tpl-claude"),
            "hidden template should NOT appear by default",
        );

        // But user-owned rows (is_seeded=0) still appear — hide only
        // affects templates.
        assert!(after.iter().any(|a| a.id == "user-a"));

        // include_hidden = true brings it back (settings panel surface).
        let included: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({ "include_hidden": true }),
        )
        .await;
        let tpl = included
            .iter()
            .find(|a| a.id == "tpl-claude")
            .expect("hidden template should appear with include_hidden=true");
        assert_eq!(tpl.user_hidden, 1);
    }

    #[tokio::test]
    async fn hide_then_unhide_round_trip() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let _: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "tpl-claude" }),
        )
        .await;
        let resp: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_UNHIDE,
            serde_json::json!({ "definition_id": "tpl-claude" }),
        )
        .await;
        assert!(resp.ok);
        // Listagents now shows it again, default-filter included.
        let agents: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({}),
        )
        .await;
        let tpl = agents
            .iter()
            .find(|a| a.id == "tpl-claude")
            .expect("unhidden template should appear");
        assert_eq!(tpl.user_hidden, 0);
    }

    #[tokio::test]
    async fn hide_rejects_user_owned_definition() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        // "user-a" is is_seeded=0 — hide must reject.
        let err = call_rpc_expect_error(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "user-a" }),
        )
        .await;
        assert!(
            err.contains("not a seeded template"),
            "wrong error: {err}"
        );
    }

    #[tokio::test]
    async fn hide_unknown_id_returns_ok_false() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let resp: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "no-such-id" }),
        )
        .await;
        assert!(!resp.ok);
    }

    #[tokio::test]
    async fn list_hidden_templates_returns_only_hidden_templates() {
        let (_state, engine, mut rx) = build_state_with_template_seed();
        // Empty initially.
        let empty: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_LIST_HIDDEN_TEMPLATES,
            serde_json::json!({}),
        )
        .await;
        assert!(empty.is_empty());

        // Hide one; expect it to surface.
        let _: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "tpl-claude" }),
        )
        .await;
        let hidden: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_LIST_HIDDEN_TEMPLATES,
            serde_json::json!({}),
        )
        .await;
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].id, "tpl-claude");
        assert_eq!(hidden[0].is_seeded, 1);
        assert_eq!(hidden[0].user_hidden, 1);
    }

    #[tokio::test]
    async fn listagents_is_seeded_filter_with_include_hidden_combines() {
        // Templates-only filter + include_hidden = the settings panel's
        // canonical query if it ever wanted the full template universe.
        // Without include_hidden + is_seeded=1 the hidden ones drop out.
        let (_state, engine, mut rx) = build_state_with_template_seed();
        let _: crate::backend::rpc_types::AgentDefHideResult = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_AGENT_DEF_HIDE,
            serde_json::json!({ "definition_id": "tpl-claude" }),
        )
        .await;
        let templates_visible: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({ "is_seeded": 1 }),
        )
        .await;
        assert!(
            !templates_visible.iter().any(|a| a.id == "tpl-claude"),
            "hidden template should be excluded from is_seeded=1 default query",
        );
        let templates_all: Vec<AgentDefinition> = call_rpc(
            &engine,
            &mut rx,
            crate::backend::rpc_types::COMMAND_LIST_AGENTS,
            serde_json::json!({ "is_seeded": 1, "include_hidden": true }),
        )
        .await;
        assert!(templates_all.iter().any(|a| a.id == "tpl-claude"));
    }
}
