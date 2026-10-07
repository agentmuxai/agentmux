// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `tests`, moved out of server/native_memory_handlers.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4).

use super::*;

// Process-global env access (AGENTMUX_SHARED_DIR / AGENTMUX_HOME_OVERRIDE
// both feed registry::paths::resolve_global_shared_root) — a module-local
// lock only serializes tests within THIS file; registry::paths's own
// test module touches the same resolution path and already uses this
// crate-wide lock for exactly that reason (see test_support.rs's doc
// comment). Reusing it here avoids reintroducing the cross-module race
// it was built to prevent.
use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;

#[test]
fn config_dir_for_default_identity_is_empty() {
    // Unbound / "default" agents use the shared default home; we return an
    // empty string so memory_dir_for_cwd applies its own providers/claude
    // fallback rather than duplicating it here.
    assert_eq!(claude_config_dir_for_identity(None), "");
    assert_eq!(claude_config_dir_for_identity(Some("")), "");
    assert_eq!(claude_config_dir_for_identity(Some("default")), "");
}

#[test]
fn config_dir_for_bound_identity_points_at_bundle() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
    std::env::set_var("AGENTMUX_SHARED_DIR", "/home/u/.agentmux/shared");

    let got = claude_config_dir_for_identity(Some("bundle-x"));
    let want = PathBuf::from("/home/u/.agentmux/shared")
        .join("identities")
        .join("bundle-x")
        .join("claude")
        .to_string_lossy()
        .to_string();
    assert_eq!(got, want);

    match prev {
        Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
        None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
    }
}

// SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md follow-up: confirmed
// live — a real agent named "AgentY" (routing slug "agenty") got
// "memory: agent agenty not found" from `MemoryList`, because this
// lookup used to compare the slug against the registry's own
// display-cased `instance_name` with raw string equality.
#[test]
fn find_active_registry_record_by_slug_resolves_a_mixed_case_display_name() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());

    let registry_dir = tmp.path().join("agents").join("registry");
    let registry = crate::registry::Registry::open(registry_dir).unwrap();
    registry
        .upsert(&crate::registry::NamedAgentRecord {
            schema_version: 1,
            data: crate::registry::NamedAgentRecordV1 {
                instance_id: "inst-agenty".to_string(),
                instance_name: "AgentY".to_string(),
                definition_id: "def-agenty".to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: "agenty-0629j".to_string(),
                source_agents_base: None,
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "test".to_string(),
                last_launched_by_version: "test".to_string(),
            },
        })
        .unwrap();

    let found = find_active_registry_record_by_slug("agenty");
    assert!(found.is_some(), "must resolve via the slug-normalized fallback");
    assert_eq!(found.unwrap().data.instance_name, "AgentY");

    let not_found = find_active_registry_record_by_slug("someone-else");
    assert!(not_found.is_none(), "an unrelated slug must not match by coincidence");

    match prev {
        Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
        None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
    }
}

// ---- Durable sync integration tests ----------------------------------
// SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md §5: simulate two
// "channels" against the same agent.id by pointing each AppState's
// mstore at a different working_directory (so memory_dir_for_cwd
// resolves two different live paths) while sharing one id_store — the
// same topology production uses (each channel's own objects.db caches
// the same global AgentDefinition.id; one shared store.db backs id_store).

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::storage::store::Store;
use crate::backend::storage::AgentDefinition;
use crate::backend::rpc_types::{RpcMessage};

fn agent_def(id: &str, working_directory: &str) -> AgentDefinition {
    AgentDefinition {
        conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
        id: id.to_string(),
        slug: id.to_string(),
        name: "Test Agent".to_string(),
        icon: String::new(),
        provider: "claude".to_string(),
        description: String::new(),
        working_directory: working_directory.to_string(),
        shell: String::new(),
        provider_flags: String::new(),
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: "host".to_string(),
        environment: String::new(),
        agent_bus_id: String::new(),
        is_seeded: 0,
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
        memory_id: String::new(),
    }
}

/// Build a channel's AppState: its own per-channel mstore (holding the
/// agent definition, keyed by the same `agent_id` every channel shares)
/// plus the given shared `id_store` (the durable mirror).
/// `claude_config_dir` must be a per-test temp directory — an empty
/// value would make `memory_dir_for_cwd` fall back to the REAL
/// `~/.agentmux/shared/providers/claude/`, writing test fixtures into
/// the developer's actual home directory (caught live: a prior version
/// of these tests left `-work-channel-a/` behind under the real home).
fn build_channel_state(
    agent_id: &str,
    working_directory: &str,
    claude_config_dir: &std::path::Path,
    id_store: Arc<Store>,
) -> (Arc<WshRpcEngine>, tokio::sync::mpsc::UnboundedReceiver<RpcMessage>) {
    build_channel_state_with_broker(
        agent_id,
        working_directory,
        claude_config_dir,
        id_store,
        Arc::new(crate::backend::mps::Broker::new()),
    )
}

/// Same as [`build_channel_state`], but lets the caller supply (and so
/// later inspect, via `RecordingWpsClient`) the `Broker` the registered
/// handlers publish `agent:memory:changed:*` events through — the
/// existing function's default broker has no observable client wired in,
/// so tests asserting on those publishes need this variant instead.
fn build_channel_state_with_broker(
    agent_id: &str,
    working_directory: &str,
    claude_config_dir: &std::path::Path,
    id_store: Arc<Store>,
    broker: Arc<crate::backend::mps::Broker>,
) -> (Arc<WshRpcEngine>, tokio::sync::mpsc::UnboundedReceiver<RpcMessage>) {
    let mstore = Arc::new(Store::open_in_memory().unwrap());
    let mut def = agent_def(agent_id, working_directory);
    mstore.agent_def_insert(&mut def).unwrap();
    mstore
        .agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: agent_id.to_string(),
            content_type: "env".to_string(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", claude_config_dir.display()),
            updated_at: 0,
        })
        .unwrap();

    let mut state = crate::server::tests::test_state();
    state.mstore = mstore.clone();
    state.id_store = id_store;
    state.broker = broker;

    let (engine, rx) = WshRpcEngine::new();
    register_native_memory_handlers(&engine, &state);
    (engine, rx)
}

async fn call_rpc<T: serde::de::DeserializeOwned>(
    engine: &Arc<WshRpcEngine>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
    command: &str,
    data: serde_json::Value,
) -> T {
    let req_id = format!("test-{}", uuid::Uuid::new_v4());
    let msg = RpcMessage {
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
    serde_json::from_value(resp.data.unwrap_or(serde_json::Value::Null)).expect("response deserialize")
}

async fn call_rpc_expect_error(
    engine: &Arc<WshRpcEngine>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
    command: &str,
    data: serde_json::Value,
) -> String {
    let req_id = format!("test-{}", uuid::Uuid::new_v4());
    let msg = RpcMessage {
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
    assert!(!resp.error.is_empty(), "expected error, got success: {:?}", resp.data);
    resp.error
}

#[tokio::test]
async fn a_file_written_from_one_channel_is_visible_from_another() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config_a = tempfile::tempdir().unwrap();
    let config_b = tempfile::tempdir().unwrap();

    let (engine_a, mut rx_a) = build_channel_state("agent-shared-1", "/work/channel-a", config_a.path(), shared_id_store.clone());
    let (engine_b, mut rx_b) = build_channel_state("agent-shared-1", "/work/channel-b", config_b.path(), shared_id_store.clone());

    call_rpc::<Option<serde_json::Value>>(
        &engine_a,
        &mut rx_a,
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({
            "agent_id": "agent-shared-1",
            "filename": "MEMORY.md",
            "content": "written from channel A",
        }),
    )
    .await;

    // Channel B's live FS never had this file — list must still surface
    // it (via the shared mirror), and read_file must return its content.
    let listed: NativeMemoryListResult = call_rpc(
        &engine_b,
        &mut rx_b,
        COMMAND_NATIVE_MEMORY_LIST,
        serde_json::json!({ "agent_id": "agent-shared-1" }),
    )
    .await;
    assert_eq!(listed.files.len(), 1, "channel B must see channel A's mirrored file");
    assert_eq!(listed.files[0].filename, "MEMORY.md");

    let read: NativeMemoryReadFileResult = call_rpc(
        &engine_b,
        &mut rx_b,
        COMMAND_NATIVE_MEMORY_READ_FILE,
        serde_json::json!({ "agent_id": "agent-shared-1", "filename": "MEMORY.md" }),
    )
    .await;
    assert_eq!(read.content, "written from channel A");
}

// ---- base_sha256 (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4) ----

fn sha256_hex(s: &str) -> String {
    crate::backend::storage::agent_native_memory_versions::content_hash(s)
}

async fn read_content(
    engine: &Arc<WshRpcEngine>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<RpcMessage>,
    agent_id: &str,
) -> String {
    let read: NativeMemoryReadFileResult = call_rpc(
        engine,
        rx,
        COMMAND_NATIVE_MEMORY_READ_FILE,
        serde_json::json!({ "agent_id": agent_id, "filename": "MEMORY.md" }),
    )
    .await;
    read.content
}

#[tokio::test]
async fn write_with_a_matching_base_sha256_is_applied() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-base-1", "/work/base-1", config.path(), id_store);

    call_rpc::<Option<serde_json::Value>>(&engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE, serde_json::json!({
        "agent_id": "agent-base-1", "filename": "MEMORY.md", "content": "v1",
    }))
    .await;
    call_rpc::<Option<serde_json::Value>>(&engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE, serde_json::json!({
        "agent_id": "agent-base-1", "filename": "MEMORY.md", "content": "v2",
        "base_sha256": sha256_hex("v1"),
    }))
    .await;
    assert_eq!(read_content(&engine, &mut rx, "agent-base-1").await, "v2");
}

#[tokio::test]
async fn write_with_a_stale_base_sha256_is_refused_as_a_conflict_and_writes_nothing() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-base-2", "/work/base-2", config.path(), id_store.clone());

    call_rpc::<Option<serde_json::Value>>(&engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE, serde_json::json!({
        "agent_id": "agent-base-2", "filename": "MEMORY.md", "content": "v1",
    }))
    .await;
    // Someone else writes while the human is editing a draft based on v1.
    call_rpc::<Option<serde_json::Value>>(&engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE, serde_json::json!({
        "agent_id": "agent-base-2", "filename": "MEMORY.md", "content": "agent wrote this",
    }))
    .await;
    let versions_before = id_store.agent_native_memory_version_list("agent-base-2", "MEMORY.md").unwrap().len();

    let err = call_rpc_expect_error(&engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE, serde_json::json!({
        "agent_id": "agent-base-2", "filename": "MEMORY.md", "content": "human draft",
        "base_sha256": sha256_hex("v1"),
    }))
    .await;
    assert!(err.contains("conflict:"), "a stale base must surface as a conflict, got: {err}");
    assert_eq!(read_content(&engine, &mut rx, "agent-base-2").await, "agent wrote this", "must not overwrite");
    assert_eq!(
        id_store.agent_native_memory_version_list("agent-base-2", "MEMORY.md").unwrap().len(),
        versions_before,
        "a refused write must not record a version"
    );
}

#[tokio::test]
async fn write_without_base_sha256_still_overwrites_as_before() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-base-3", "/work/base-3", config.path(), id_store);

    for content in ["v1", "v2 (no base, last writer wins)"] {
        call_rpc::<Option<serde_json::Value>>(&engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE, serde_json::json!({
            "agent_id": "agent-base-3", "filename": "MEMORY.md", "content": content,
        }))
        .await;
    }
    assert_eq!(read_content(&engine, &mut rx, "agent-base-3").await, "v2 (no base, last writer wins)");
}

#[tokio::test]
async fn write_with_a_base_for_a_file_that_no_longer_exists_is_a_conflict() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-base-4", "/work/base-4", config.path(), id_store);

    let err = call_rpc_expect_error(&engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE, serde_json::json!({
        "agent_id": "agent-base-4", "filename": "MEMORY.md", "content": "draft",
        "base_sha256": sha256_hex("v1"),
    }))
    .await;
    assert!(err.contains("conflict:") && err.contains("no longer exists"), "got: {err}");
}

#[test]
fn check_memory_base_sha256_accepts_either_hex_case() {
    let base = sha256_hex("same").to_uppercase();
    assert!(check_memory_base_sha256("MEMORY.md", &base, Some("same")).is_ok());
}

#[tokio::test]
async fn list_reports_the_files_real_mtime_for_a_mirror_only_entry() {
    // reagent P1 on PR #2459 (fifth pass): a mirror-only listing entry
    // (no live copy on this channel) must report the FILE's real
    // last-modified time (last_seen_mtime_ms), not the mirror row's own
    // sync timestamp (updated_at) — those are two different clocks that
    // just happen to be close together right after a write.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();

    // A deliberately old, obviously-not-"just synced" mtime — upsert's
    // own now_ms() for updated_at will always land far later than this.
    const REAL_FILE_MTIME_MS: i64 = 12_345;
    shared_id_store
        .agent_native_memory_upsert("agent-mtime", "MEMORY.md", "content", None, "/elsewhere", 7, REAL_FILE_MTIME_MS)
        .unwrap();

    let (engine, mut rx) = build_channel_state("agent-mtime", "/work/channel-a", config.path(), shared_id_store);

    let listed: NativeMemoryListResult = call_rpc(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_LIST,
        serde_json::json!({ "agent_id": "agent-mtime" }),
    )
    .await;
    assert_eq!(listed.files.len(), 1);
    assert_eq!(
        listed.files[0].modified_at, REAL_FILE_MTIME_MS,
        "mirror-only entries must report the file's real mtime, not the mirror's own sync timestamp"
    );
}

#[tokio::test]
async fn live_fs_content_wins_over_a_stale_mirror_row() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config_a = tempfile::tempdir().unwrap();

    let (engine_a, mut rx_a) = build_channel_state("agent-shared-2", "/work/channel-a", config_a.path(), shared_id_store.clone());

    call_rpc::<Option<serde_json::Value>>(
        &engine_a,
        &mut rx_a,
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-shared-2", "filename": "MEMORY.md", "content": "v1" }),
    )
    .await;
    call_rpc::<Option<serde_json::Value>>(
        &engine_a,
        &mut rx_a,
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-shared-2", "filename": "MEMORY.md", "content": "v2 — freshest" }),
    )
    .await;

    // Directly stamp a stale mirror row behind the live FS's back — the
    // read path must still prefer the live file, not this stale row.
    shared_id_store
        .agent_native_memory_upsert("agent-shared-2", "MEMORY.md", "stale mirror content", None, "/nowhere", "stale mirror content".len() as i64, 0)
        .unwrap();

    let read: NativeMemoryReadFileResult = call_rpc(
        &engine_a,
        &mut rx_a,
        COMMAND_NATIVE_MEMORY_READ_FILE,
        serde_json::json!({ "agent_id": "agent-shared-2", "filename": "MEMORY.md" }),
    )
    .await;
    assert_eq!(read.content, "v2 — freshest", "live FS must win over a stale mirror row");
}

#[tokio::test]
async fn read_file_errors_when_absent_from_both_live_fs_and_mirror() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config_a = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-shared-3", "/work/channel-a", config_a.path(), shared_id_store);

    let err = call_rpc_expect_error(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_READ_FILE,
        serde_json::json!({ "agent_id": "agent-shared-3", "filename": "MEMORY.md" }),
    )
    .await;
    assert!(err.contains("not found"), "unexpected error: {err}");
}

#[tokio::test]
async fn read_file_rejects_a_non_regular_file_instead_of_falling_back_to_the_mirror() {
    // reagent P1 on PR #2459 (second pass): a path that exists but isn't a
    // regular file (e.g. a directory landed at the expected filename) must
    // be rejected explicitly — collapsing it into "absent, fall back to
    // mirror" could serve stale mirrored content for a path that actually
    // exists but is the wrong type.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    // Pre-mirror some content, so a bug that treats this as "absent" would
    // wrongly succeed by serving it instead of erroring.
    shared_id_store
        .agent_native_memory_upsert("agent-wrongtype", "MEMORY.md", "mirrored content", None, "/elsewhere", "mirrored content".len() as i64, 0)
        .unwrap();
    let (engine, mut rx) = build_channel_state("agent-wrongtype", "/work/channel-a", config.path(), shared_id_store);

    let memory_dir = config.path().join("projects").join("-work-channel-a").join("memory");
    std::fs::create_dir_all(memory_dir.join("MEMORY.md")).unwrap(); // a directory, not a file

    let err = call_rpc_expect_error(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_READ_FILE,
        serde_json::json!({ "agent_id": "agent-wrongtype", "filename": "MEMORY.md" }),
    )
    .await;
    assert!(err.contains("not a regular file"), "unexpected error: {err}");
}

#[tokio::test]
async fn list_propagates_a_real_read_dir_error_instead_of_treating_it_as_empty() {
    // reagent P2 on PR #2459: `std::fs::read_dir(&memory_dir).ok()` used to
    // swallow every error (permissions, I/O — not just the legitimate
    // "directory doesn't exist yet" case), silently reporting an empty
    // listing instead of surfacing a real access problem. Force a
    // non-NotFound read_dir error by making the "memory" path a regular
    // file instead of a directory.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();

    // memory_dir_for_cwd sanitizes "/work/channel-a" to "-work-channel-a".
    let projects_dir = config.path().join("projects").join("-work-channel-a");
    std::fs::create_dir_all(&projects_dir).unwrap();
    std::fs::write(projects_dir.join("memory"), b"not a directory").unwrap();

    let (engine, mut rx) = build_channel_state("agent-baddir", "/work/channel-a", config.path(), shared_id_store);

    let err = call_rpc_expect_error(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_LIST,
        serde_json::json!({ "agent_id": "agent-baddir" }),
    )
    .await;
    assert!(err.contains("read_dir"), "expected a propagated read_dir error, got: {err}");
}

#[tokio::test]
async fn list_does_not_re_upsert_an_unchanged_file_on_a_second_call() {
    // reagent P1 on PR #2459: list() must not do a full-content read +
    // SQLite write for every file on every call — only for a file whose
    // size differs from what's already mirrored. Two back-to-back list()
    // calls on an untouched file should produce exactly one mirror write.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-unchanged", "/work/channel-a", config.path(), shared_id_store.clone());

    let memory_dir = config.path().join("projects").join("-work-channel-a").join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(memory_dir.join("MEMORY.md"), "stable content").unwrap();

    let _: NativeMemoryListResult = call_rpc(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_LIST,
        serde_json::json!({ "agent_id": "agent-unchanged" }),
    )
    .await;
    let first_updated_at = shared_id_store
        .agent_native_memory_list_meta("agent-unchanged")
        .unwrap()[0]
        .updated_at;

    let _: NativeMemoryListResult = call_rpc(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_LIST,
        serde_json::json!({ "agent_id": "agent-unchanged" }),
    )
    .await;
    let second_updated_at = shared_id_store
        .agent_native_memory_list_meta("agent-unchanged")
        .unwrap()[0]
        .updated_at;

    assert_eq!(
        first_updated_at, second_updated_at,
        "an unchanged file must not be re-upserted into the mirror on a second list() call"
    );
}

#[tokio::test]
async fn list_detects_a_same_size_content_change_via_mtime() {
    // reagent P1 on PR #2459: comparing size alone would miss a same-byte-
    // length edit (e.g. correcting a typo), leaving the mirror serving
    // stale content forever to a channel that never had a live copy of
    // its own to self-correct with. Confirm the mtime check catches it.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-samesize", "/work/channel-a", config.path(), shared_id_store.clone());

    let memory_dir = config.path().join("projects").join("-work-channel-a").join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let file = memory_dir.join("MEMORY.md");
    std::fs::write(&file, "content A!").unwrap();

    let _: NativeMemoryListResult = call_rpc(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_LIST,
        serde_json::json!({ "agent_id": "agent-samesize" }),
    )
    .await;
    assert_eq!(
        shared_id_store.agent_native_memory_read("agent-samesize", "MEMORY.md").unwrap(),
        Some("content A!".to_string())
    );

    // Same byte length, different content, comfortably past mtime resolution.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    std::fs::write(&file, "content B!").unwrap();
    assert_eq!(file.metadata().unwrap().len(), "content A!".len() as u64);

    let _: NativeMemoryListResult = call_rpc(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_LIST,
        serde_json::json!({ "agent_id": "agent-samesize" }),
    )
    .await;
    assert_eq!(
        shared_id_store.agent_native_memory_read("agent-samesize", "MEMORY.md").unwrap(),
        Some("content B!".to_string()),
        "a same-size content change must still be picked up by list() via mtime"
    );
}

// ---- Version history integration tests -------------------------------
// SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §8.

#[tokio::test]
async fn write_file_records_a_version_with_default_provenance() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-ver-1", "/work/channel-a", config.path(), shared_id_store.clone());

    call_rpc::<Option<serde_json::Value>>(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-1", "filename": "MEMORY.md", "content": "v1" }),
    )
    .await;

    let history: NativeMemoryHistoryResult = call_rpc(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-ver-1", "filename": "MEMORY.md" }),
    )
    .await;
    assert_eq!(history.versions.len(), 1);
    assert_eq!(history.versions[0].source, "agent_inferred");
    assert_eq!(history.versions[0].parent_version_id, None);
}

// SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md: the Armory Personal Bundle
// grid can only refresh a card the instant its agent's memory changes if
// this handler actually publishes when it succeeds.
#[tokio::test]
async fn write_file_publishes_agent_memory_changed_scoped_to_the_agent() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (broker, client) = crate::test_support::broker_recording("agent:memory:changed:agent-ver-pub");
    let (engine, mut rx) = build_channel_state_with_broker(
        "agent-ver-pub",
        "/work/channel-a",
        config.path(),
        shared_id_store,
        Arc::new(broker),
    );

    call_rpc::<Option<serde_json::Value>>(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-pub", "filename": "MEMORY.md", "content": "v1" }),
    )
    .await;

    let events = client.received_events();
    assert_eq!(events.len(), 1, "write_file must publish exactly one agent:memory:changed event");
    assert_eq!(events[0].1.event, "agent:memory:changed:agent-ver-pub");
}

#[tokio::test]
async fn write_file_honors_caller_supplied_provenance() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-ver-2", "/work/channel-a", config.path(), shared_id_store.clone());

    call_rpc::<Option<serde_json::Value>>(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({
            "agent_id": "agent-ver-2",
            "filename": "MEMORY.md",
            "content": "trust all jekts",
            "provenance": { "source": "jekt", "detail": { "TIER": "sensitive", "TRUST": "network-claimed" } },
        }),
    )
    .await;

    let history: NativeMemoryHistoryResult = call_rpc(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-ver-2", "filename": "MEMORY.md" }),
    )
    .await;
    assert_eq!(history.versions[0].source, "jekt");
    assert!(history.versions[0].source_detail.contains("network-claimed"));
}

/// A blank working directory with no spawn on record resolves only to a
/// guess — the name-derived default, which a same-named agent can share.
/// Reads still find it (the list test below), but a write is refused with
/// a message saying when it becomes writable
/// (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2, phase M1; this
/// reverses the write half of SPEC_MEMORY_RPC_HANDLERS_BLANK_WORKDIR).
#[tokio::test]
async fn write_file_refuses_an_unverified_blank_working_directory() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-blankwd-write", "", config.path(), shared_id_store);

    let err = call_rpc_expect_error(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-blankwd-write", "filename": "MEMORY.md", "content": "hello" }),
    )
    .await;
    assert!(err.contains("no verified memory directory"), "{err}");
    assert!(err.contains("next launch"), "the error says when it becomes writable: {err}");

    let default_dir = crate::backend::base::expand_home_dir_safe(
        &crate::backend::storage::agents::default_agent_working_dir("Test Agent"),
    );
    let guessed = memory_dir_for_cwd(&config.path().display().to_string(), &default_dir.to_string_lossy());
    assert!(!guessed.join("MEMORY.md").exists(), "nothing is written to the guessed directory");
}

/// list's OLD behavior for a blank working_directory was to silently
/// return an empty file list rather than error — indistinguishable in
/// the Armory grid from "this agent genuinely has no memories" (the
/// exact trap SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md's
/// four-state card design exists to avoid). This proves list now finds
/// files that were written directly to the derived-default directory —
/// i.e. files that existed on disk all along, that list was previously
/// silently failing to find, not files this test manufactures via
/// write_file (which now shares the same fixed resolver and would trivially
/// "work" even if list's OWN resolution were still broken).
#[tokio::test]
async fn list_finds_files_already_on_disk_at_the_derived_default_dir_for_a_blank_workdir_agent() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();

    let default_dir = crate::backend::base::expand_home_dir_safe(
        &crate::backend::storage::agents::default_agent_working_dir("Test Agent"),
    );
    let memory_dir = memory_dir_for_cwd(&config.path().display().to_string(), &default_dir.to_string_lossy());
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(memory_dir.join("PRE_EXISTING.md"), "written directly to disk, not via this RPC").unwrap();

    let (engine, mut rx) = build_channel_state("agent-blankwd-preexisting", "", config.path(), shared_id_store);
    let listed: NativeMemoryListResult = call_rpc(
        &engine,
        &mut rx,
        COMMAND_NATIVE_MEMORY_LIST,
        serde_json::json!({ "agent_id": "agent-blankwd-preexisting" }),
    )
    .await;
    assert_eq!(
        listed.files.len(),
        1,
        "list must find a file that genuinely exists on disk at the derived-default dir, not report empty"
    );
    assert_eq!(listed.files[0].filename, "PRE_EXISTING.md");
}

#[tokio::test]
async fn revert_refuses_an_unverified_blank_working_directory() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-blankwd-revert", "", config.path(), shared_id_store.clone());
    let v1 = shared_id_store
        .agent_native_memory_version_insert("agent-blankwd-revert", "MEMORY.md", "v1", "agent", "{}", "")
        .unwrap();

    let err = call_rpc_expect_error(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_REVERT,
        serde_json::json!({ "agent_id": "agent-blankwd-revert", "filename": "MEMORY.md", "target_version_id": v1.id }),
    ).await;
    assert!(err.contains("no verified memory directory"), "{err}");
}

#[tokio::test]
async fn history_lists_newest_first_with_parent_chain() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-ver-3", "/work/channel-a", config.path(), shared_id_store.clone());

    call_rpc::<Option<serde_json::Value>>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-3", "filename": "MEMORY.md", "content": "v1" }),
    ).await;
    call_rpc::<Option<serde_json::Value>>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-3", "filename": "MEMORY.md", "content": "v2" }),
    ).await;

    let history: NativeMemoryHistoryResult = call_rpc(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-ver-3", "filename": "MEMORY.md" }),
    ).await;
    assert_eq!(history.versions.len(), 2);
    assert_eq!(history.versions[0].parent_version_id, Some(history.versions[1].id.clone()));
}

#[tokio::test]
async fn diff_shows_added_and_removed_lines() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-ver-4", "/work/channel-a", config.path(), shared_id_store.clone());

    call_rpc::<Option<serde_json::Value>>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-4", "filename": "MEMORY.md", "content": "line a\nline b" }),
    ).await;
    call_rpc::<Option<serde_json::Value>>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-4", "filename": "MEMORY.md", "content": "line a\nline c" }),
    ).await;

    let history: NativeMemoryHistoryResult = call_rpc(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-ver-4", "filename": "MEMORY.md" }),
    ).await;
    let (newest, oldest) = (&history.versions[0], &history.versions[1]);

    let diff: NativeMemoryDiffResult = call_rpc(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_DIFF,
        serde_json::json!({ "agent_id": "agent-ver-4", "from_version_id": oldest.id, "to_version_id": newest.id }),
    ).await;
    assert!(diff.diff.contains("  line a"), "unexpected diff: {}", diff.diff);
    assert!(diff.diff.contains("- line b"), "unexpected diff: {}", diff.diff);
    assert!(diff.diff.contains("+ line c"), "unexpected diff: {}", diff.diff);
}

#[tokio::test]
async fn diff_errors_for_an_unknown_version_id() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-ver-5", "/work/channel-a", config.path(), shared_id_store.clone());

    let err = call_rpc_expect_error(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_DIFF,
        serde_json::json!({ "agent_id": "agent-ver-5", "from_version_id": "nope", "to_version_id": "also-nope" }),
    ).await;
    assert!(err.contains("not found"), "unexpected error: {err}");
}

#[tokio::test]
async fn diff_rejects_a_version_from_a_different_agent() {
    // reagent P1: from/to must both belong to the calling agent_id —
    // this is the regression test for that fix.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config_a = tempfile::tempdir().unwrap();
    let config_b = tempfile::tempdir().unwrap();
    let (engine_a, mut rx_a) = build_channel_state("agent-diff-a", "/work/channel-a", config_a.path(), shared_id_store.clone());
    let (engine_b, mut rx_b) = build_channel_state("agent-diff-b", "/work/channel-b", config_b.path(), shared_id_store.clone());

    call_rpc::<Option<serde_json::Value>>(
        &engine_a, &mut rx_a, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-diff-a", "filename": "MEMORY.md", "content": "v1" }),
    ).await;
    call_rpc::<Option<serde_json::Value>>(
        &engine_a, &mut rx_a, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-diff-a", "filename": "MEMORY.md", "content": "v2" }),
    ).await;
    let history_a: NativeMemoryHistoryResult = call_rpc(
        &engine_a, &mut rx_a, COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-diff-a", "filename": "MEMORY.md" }),
    ).await;

    let err = call_rpc_expect_error(
        &engine_b, &mut rx_b, COMMAND_NATIVE_MEMORY_DIFF,
        serde_json::json!({
            "agent_id": "agent-diff-b",
            "from_version_id": history_a.versions[1].id,
            "to_version_id": history_a.versions[0].id,
        }),
    ).await;
    assert!(err.contains("do not belong to"), "unexpected error: {err}");
}

#[tokio::test]
async fn diff_rejects_two_versions_of_different_files() {
    // reagent P2: ownership alone isn't enough — from/to must also be
    // versions of the SAME file, or the "diff" is a meaningless
    // line-by-line comparison of two unrelated files.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-diff-files", "/work/channel-a", config.path(), shared_id_store.clone());

    call_rpc::<Option<serde_json::Value>>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-diff-files", "filename": "a.md", "content": "content a" }),
    ).await;
    call_rpc::<Option<serde_json::Value>>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-diff-files", "filename": "b.md", "content": "content b" }),
    ).await;
    let history_a: NativeMemoryHistoryResult = call_rpc(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-diff-files", "filename": "a.md" }),
    ).await;
    let history_b: NativeMemoryHistoryResult = call_rpc(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-diff-files", "filename": "b.md" }),
    ).await;

    let err = call_rpc_expect_error(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_DIFF,
        serde_json::json!({
            "agent_id": "agent-diff-files",
            "from_version_id": history_a.versions[0].id,
            "to_version_id": history_b.versions[0].id,
        }),
    ).await;
    assert!(err.contains("different files"), "unexpected error: {err}");
}

#[tokio::test]
async fn revert_writes_a_new_version_and_restores_live_content_without_deleting_history() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    let (engine, mut rx) = build_channel_state("agent-ver-6", "/work/channel-a", config.path(), shared_id_store.clone());

    call_rpc::<Option<serde_json::Value>>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md", "content": "good content" }),
    ).await;
    call_rpc::<Option<serde_json::Value>>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md", "content": "fabricated content" }),
    ).await;

    let history_before: NativeMemoryHistoryResult = call_rpc(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md" }),
    ).await;
    assert_eq!(history_before.versions.len(), 2);
    let good_version_id = history_before.versions[1].id.clone(); // oldest = "good content"

    let revert: NativeMemoryRevertResult = call_rpc(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_REVERT,
        serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md", "target_version_id": good_version_id }),
    ).await;
    assert_eq!(revert.version.source, "revert");

    // Live content (and mirror) must now read "good content" again.
    let read: NativeMemoryReadFileResult = call_rpc(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_READ_FILE,
        serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md" }),
    ).await;
    assert_eq!(read.content, "good content");

    // History must now have 3 rows (append-only — the fabricated
    // version is still there, just no longer latest), not 2.
    let history_after: NativeMemoryHistoryResult = call_rpc(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-ver-6", "filename": "MEMORY.md" }),
    ).await;
    assert_eq!(history_after.versions.len(), 3, "revert must never delete or rewrite prior versions");
}

// SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md — same rationale as
// write_file's own publish test above.
#[tokio::test]
async fn revert_publishes_agent_memory_changed_scoped_to_the_agent() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config = tempfile::tempdir().unwrap();
    // Write with the default (unobserved) broker first — only the revert
    // publish itself is under test here.
    let (engine, mut rx) = build_channel_state("agent-ver-revpub", "/work/channel-a", config.path(), shared_id_store.clone());
    call_rpc::<Option<serde_json::Value>>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-revpub", "filename": "MEMORY.md", "content": "v1" }),
    ).await;
    let history: NativeMemoryHistoryResult = call_rpc(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-ver-revpub", "filename": "MEMORY.md" }),
    ).await;
    let v1_id = history.versions[0].id.clone();
    call_rpc::<Option<serde_json::Value>>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-revpub", "filename": "MEMORY.md", "content": "v2" }),
    ).await;

    // Now rebuild the channel on an OBSERVED broker for the revert call
    // itself, against the same shared id_store so the version/history
    // above is still visible.
    let (broker, client) = crate::test_support::broker_recording("agent:memory:changed:agent-ver-revpub");
    let (engine, mut rx) = build_channel_state_with_broker(
        "agent-ver-revpub",
        "/work/channel-a",
        config.path(),
        shared_id_store,
        Arc::new(broker),
    );
    call_rpc::<NativeMemoryRevertResult>(
        &engine, &mut rx, COMMAND_NATIVE_MEMORY_REVERT,
        serde_json::json!({ "agent_id": "agent-ver-revpub", "filename": "MEMORY.md", "target_version_id": v1_id }),
    ).await;

    let events = client.received_events();
    assert_eq!(events.len(), 1, "revert must publish exactly one agent:memory:changed event");
    assert_eq!(events[0].1.event, "agent:memory:changed:agent-ver-revpub");
}

#[tokio::test]
async fn revert_rejects_a_version_belonging_to_a_different_agent() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let shared_id_store = Arc::new(Store::open_shared(tmp.path()).unwrap());
    let config_a = tempfile::tempdir().unwrap();
    let config_b = tempfile::tempdir().unwrap();
    let (engine_a, mut rx_a) = build_channel_state("agent-ver-7a", "/work/channel-a", config_a.path(), shared_id_store.clone());
    let (engine_b, mut rx_b) = build_channel_state("agent-ver-7b", "/work/channel-b", config_b.path(), shared_id_store.clone());

    call_rpc::<Option<serde_json::Value>>(
        &engine_a, &mut rx_a, COMMAND_NATIVE_MEMORY_WRITE_FILE,
        serde_json::json!({ "agent_id": "agent-ver-7a", "filename": "MEMORY.md", "content": "agent a's content" }),
    ).await;
    let history_a: NativeMemoryHistoryResult = call_rpc(
        &engine_a, &mut rx_a, COMMAND_NATIVE_MEMORY_HISTORY,
        serde_json::json!({ "agent_id": "agent-ver-7a", "filename": "MEMORY.md" }),
    ).await;

    let err = call_rpc_expect_error(
        &engine_b, &mut rx_b, COMMAND_NATIVE_MEMORY_REVERT,
        serde_json::json!({ "agent_id": "agent-ver-7b", "filename": "MEMORY.md", "target_version_id": history_a.versions[0].id }),
    ).await;
    assert!(err.contains("does not belong to"), "unexpected error: {err}");
}

#[test]
fn line_diff_marks_context_removed_and_added_lines() {
    let diff = line_diff("a\nb\nc", "a\nx\nc");
    assert_eq!(diff, "  a\n- b\n+ x\n  c\n");
}

#[test]
fn line_diff_handles_identical_content() {
    assert_eq!(line_diff("same", "same"), "  same\n");
}

// reagent P2 on PR #2674 (re-review): line_diff was rewritten from
// vec![vec![0usize; m + 1]; n + 1] (n+1 separate heap allocations) to a
// single flat Vec indexed manually via `i * cols + j` — a lopsided
// shape (one side much longer than the other) exercises exactly the
// index arithmetic that refactor could get wrong, so cover it with an
// asymmetric case in both directions rather than only the roughly
// square cases above.
#[test]
fn line_diff_handles_a_lopsided_shape_in_both_directions() {
    assert_eq!(line_diff("a\nb\nc\nd\ne", "c"), "- a\n- b\n  c\n- d\n- e\n");
    assert_eq!(line_diff("c", "a\nb\nc\nd\ne"), "+ a\n+ b\n  c\n+ d\n+ e\n");
}

// A registry record is a guess about where an agent's memory is — its
// `identity_id` can be stale, which once attributed another agent's
// files to this one. Only verified directories are enumerated
// (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2, phase M1).
#[tokio::test]
async fn list_all_memory_targets_leaves_out_registry_only_agents() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());

    let registry_dir = tmp.path().join("agents").join("registry");
    let registry = crate::registry::Registry::open(registry_dir).unwrap();
    registry
        .upsert(&crate::registry::NamedAgentRecord {
            schema_version: 1,
            data: crate::registry::NamedAgentRecordV1 {
                instance_id: "inst-live-only".to_string(),
                instance_name: "LiveOnly".to_string(),
                definition_id: "def-live-only".to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: "live-only-proj".to_string(),
                source_agents_base: Some(tmp.path().join("agents").to_string_lossy().to_string()),
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "test".to_string(),
                last_launched_by_version: "test".to_string(),
            },
        })
        .unwrap();

    let state = crate::server::tests::test_state();
    let targets = list_all_memory_targets(&state.mstore);
    assert!(
        !targets.iter().any(|(id, _)| id == "def-live-only"),
        "a registry-only agent is a guess, not a verified directory: {targets:?}"
    );

    match prev {
        Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
        None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
    }
}

// A db_agents row is the more authoritative, complete source for an
// agent that has one — its own memory dir resolution must win over a
// registry-reconstructed guess for the same logical agent, and the
// agent must be enumerated exactly once, not twice.
#[tokio::test]
async fn list_all_memory_targets_dedupes_an_agent_present_in_both_db_agents_and_the_registry() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());

    let state = crate::server::tests::test_state();
    let config_dir = tempfile::tempdir().unwrap();
    let mut def = crate::backend::storage::AgentDefinition {
        conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
        id: "dup-agent".to_string(),
        slug: "dup-agent".to_string(),
        name: "Test".to_string(),
        icon: String::new(),
        provider: "claude".to_string(),
        description: String::new(),
        working_directory: "/work/dup-agent".to_string(),
        shell: String::new(),
        provider_flags: String::new(),
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: "host".to_string(),
        environment: String::new(),
        agent_bus_id: String::new(),
        is_seeded: 0,
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
        memory_id: String::new(),
    };
    state.mstore.agent_def_insert(&mut def).unwrap();
    state
        .mstore
        .agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: "dup-agent".to_string(),
            content_type: "env".to_string(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", config_dir.path().display()),
            updated_at: 0,
        })
        .unwrap();
    let expected_dir = memory_dir_for_agent_by_id(&state.mstore, &def).unwrap();

    let registry_dir = tmp.path().join("agents").join("registry");
    let registry = crate::registry::Registry::open(registry_dir).unwrap();
    registry
        .upsert(&crate::registry::NamedAgentRecord {
            schema_version: 1,
            data: crate::registry::NamedAgentRecordV1 {
                instance_id: "inst-dup".to_string(),
                instance_name: "dup-agent".to_string(),
                definition_id: "dup-agent".to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                // Deliberately a different working dir than db_agents'
                // own row — proves the db_agents-derived entry wins
                // rather than being silently overwritten.
                working_dir: "different-registry-guess".to_string(),
                source_agents_base: Some(tmp.path().join("agents").to_string_lossy().to_string()),
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "test".to_string(),
                last_launched_by_version: "test".to_string(),
            },
        })
        .unwrap();

    let targets = list_all_memory_targets(&state.mstore);
    let matches: Vec<_> = targets.iter().filter(|(id, _)| id == "dup-agent").collect();
    assert_eq!(matches.len(), 1, "must not enumerate the same agent twice: {targets:?}");
    assert_eq!(matches[0].1, expected_dir, "the db_agents-derived dir must win over the registry's");

    match prev {
        Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
        None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
    }
}

/// #3603: an agent with no `CLAUDE_CONFIG_DIR` in its env but a linked
/// Claude OAuth account runs Claude in that account's dir, so its
/// memories are there — not under the shared default.
#[test]
fn memory_dir_follows_the_linked_oauth_account_when_env_has_none() {
    use crate::backend::storage::identities::{IdentityAccount, SecretRef};
    let store = std::sync::Arc::new(Store::open_in_memory().unwrap());
    let mut def = crate::backend::storage::agents::test_agent_def("uid-3603", "M", "claude", "agent", 1, "");
    def.slug = "m-3603".into();
    def.working_directory = "/work/m-3603".into();
    store.agent_def_insert(&mut def).unwrap();
    store
        .identity_upsert(&IdentityAccount {
            id: "acct-3603".into(),
            name: "claude-oauth".into(),
            provider: "claude".into(),
            kind: "oauth".into(),
            display_name: String::new(),
            secret_ref: SecretRef::OAuthConfigDir { dir: "/accounts/acct-3603/claude".into() },
            context: serde_json::json!({}),
            status: "valid".into(),
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();
    store.agent_identity_link("uid-3603", "acct-3603", "claude").unwrap();
    // One store plays all three roles here; an in-process OnceLock, so it
    // is attached once — other tests' agents have no links in it.
    attach_identity_stores(store.clone(), store.clone());

    let got = memory_dir_for_agent_by_id(&store, &def).unwrap();
    assert_eq!(got, memory_dir_for_cwd("/accounts/acct-3603/claude", "/work/m-3603"));
    assert_eq!(
        memory_dir_for_agent(&store, "m-3603").unwrap(),
        got,
        "the slug path resolves the same dir"
    );

    // The spawn sets CLAUDE_CONFIG_DIR to the linked account's dir over
    // any env value, so memory follows the account too.
    store
        .agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: "uid-3603".into(),
            content_type: "env".into(),
            content: "CLAUDE_CONFIG_DIR=/somewhere/else\n".into(),
            updated_at: 0,
        })
        .unwrap();
    assert_eq!(memory_dir_for_agent_by_id(&store, &def).unwrap(), got, "the account wins");

    // An agent with no link of its own inherits its template's, as the
    // spawn does (muxreview on #3630).
    let mut tpl = crate::backend::storage::agents::test_agent_def("tpl-3603", "T", "claude", "agent", 1, "");
    tpl.is_seeded = 1;
    store.agent_def_insert(&mut tpl).unwrap();
    store.agent_identity_link("tpl-3603", "acct-3603", "claude").unwrap();
    let mut child = crate::backend::storage::agents::test_agent_def("uid-3603-child", "K", "claude", "agent", 1, "");
    child.slug = "k-3603".into();
    child.parent_id = "tpl-3603".into();
    child.working_directory = "/work/k-3603".into();
    store.agent_def_insert(&mut child).unwrap();
    assert_eq!(
        memory_dir_for_agent_by_id(&store, &child).unwrap(),
        memory_dir_for_cwd("/accounts/acct-3603/claude", "/work/k-3603"),
        "the template's link"
    );

    // A non-Claude agent's linked account is not a Claude config dir:
    // memory keeps today's resolution.
    let mut codex = crate::backend::storage::agents::test_agent_def("uid-3603-codex", "C", "codex", "agent", 1, "");
    codex.slug = "c-3603".into();
    codex.working_directory = "/work/c-3603".into();
    store.agent_def_insert(&mut codex).unwrap();
    store.agent_identity_link("uid-3603-codex", "acct-3603", "codex").unwrap();
    assert_eq!(
        memory_dir_for_agent_by_id(&store, &codex).unwrap(),
        memory_dir_for_cwd("", "/work/c-3603"),
    );
}

#[test]
fn memory_dir_for_cwd_default_root_matches_spawn_layout() {
    // Empty config dir → the default isolated home under shared, with the
    // working dir sanitized the same way Claude Code encodes project dirs.
    // Assert on path components so mixed separators (Windows) don't matter.
    let dir = memory_dir_for_cwd("", "/work/proj");
    let comps: Vec<String> = dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let want_tail = ["providers", "claude", "projects", "-work-proj", "memory"];
    let tail = &comps[comps.len() - want_tail.len()..];
    assert_eq!(tail, want_tail, "unexpected memory dir tail: {comps:?}");
}


/// The non-blank path is unaffected: a real working_directory still
/// resolves straight from the instance row, without consulting the
/// registry at all.
#[test]
fn non_blank_working_directory_still_resolves_from_the_instance_row() {
    let mstore = Store::open_in_memory().unwrap();
    let mut def = agent_def("realwd-agent", "/work/proj");
    mstore.agent_def_insert(&mut def).unwrap();

    let dir = memory_dir_for_agent(&mstore, "realwd-agent").unwrap();
    let comps: Vec<String> = dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let want_tail = ["projects", "-work-proj", "memory"];
    let tail = &comps[comps.len() - want_tail.len()..];
    assert_eq!(tail, want_tail, "unexpected memory dir tail: {comps:?}");
}

/// A blank `working_directory` — the DEFAULT for a newly defined agent —
/// must resolve to the same path `agent.open` itself substitutes, not
/// fail. Before the fix `memory_dir_for_agent` returned
/// "agent <x> has no working directory" and Armory → Bundle → Personal
/// (and the MemoryList MCP tool) were broken for the common case, while
/// the agent's memory files sat on disk intact. Reproduced live:
///   memory/list failed: HTTP 500 — "memory: agent manoz has no working directory"
///
/// With no registry record present, stage 2 (the derived default) is what
/// must answer — the case Codex P1 flagged, since `agent.open` does not
/// create a registry record.
#[test]
fn blank_working_directory_resolves_to_the_same_default_agent_open_substitutes() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mstore = Store::open_in_memory().unwrap();
    let mut def = agent_def("blankwd-agent", "");
    def.name = "Blank WD Agent".to_string();
    mstore.agent_def_insert(&mut def).unwrap();

    let dir = memory_dir_for_agent(&mstore, "blankwd-agent")
        .expect("a blank working_directory must resolve, not error");

    // default_agent_working_dir("Blank WD Agent") -> ~/.agentmux/agents/blank-wd-agent,
    // expanded to the absolute path Claude records as its cwd, then named
    // as Claude names project folders (`claude_layout`'s tests pin that
    // rule against the CLI). The unexpanded form (`---agentmux-agents-…`)
    // named a folder Claude never writes (#3603 review).
    let expanded = crate::backend::base::expand_home_dir_safe("~/.agentmux/agents/blank-wd-agent");
    let folder = crate::backend::claude_layout::project_dir_name(&expanded.to_string_lossy());
    let comps: Vec<String> = dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let want_tail = ["projects", folder.as_str(), "memory"];
    let tail = &comps[comps.len() - want_tail.len()..];
    assert_eq!(tail, want_tail, "unexpected memory dir tail: {comps:?}");
}

/// The shared helper must stay byte-identical to `agent.open`'s own
/// substitution — they are the same path by contract, and drift between
/// them is precisely what broke Personal Bundle.
#[test]
fn default_agent_working_dir_matches_agent_opens_inline_derivation() {
    for name in ["Manoz", "Blank WD Agent", "Wei_Zhang-2", "Zurich Nome"] {
        let inline: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect();
        assert_eq!(
            crate::backend::storage::agents::default_agent_working_dir(name),
            format!("~/.agentmux/agents/{inline}"),
            "drifted from agent_open's derivation for {name:?}",
        );
    }
}

/// Codex P1: the registry stage must be bound to the agent's own
/// definition. `find_active_registry_record_by_slug` matches on
/// `derive_slug(instance_name)` alone, so two agents whose display names
/// slugify the same collide — resolving the WRONG agent's memory dir
/// would let list/read/write touch another agent's files. A record whose
/// `definition_id` doesn't match must be ignored, falling through to the
/// derived default instead.
#[test]
fn a_registry_record_for_a_different_definition_is_never_used() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());

    let registry =
        crate::registry::Registry::open(tmp.path().join("agents").join("registry")).unwrap();
    registry
        .upsert(&crate::registry::NamedAgentRecord {
            schema_version: 1,
            data: crate::registry::NamedAgentRecordV1 {
                instance_id: "inst-other".to_string(),
                // Slugifies to the same slug as our agent below...
                instance_name: "Collide Agent".to_string(),
                // ...but belongs to a DIFFERENT definition.
                definition_id: "def-somebody-else".to_string(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: "somebody-elses-dir".to_string(),
                source_agents_base: Some(tmp.path().to_string_lossy().to_string()),
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "test".to_string(),
                last_launched_by_version: "test".to_string(),
            },
        })
        .unwrap();

    let mstore = Store::open_in_memory().unwrap();
    let mut def = agent_def("collide-agent", "");
    def.name = "Collide Agent".to_string();
    mstore.agent_def_insert(&mut def).unwrap();

    let dir = memory_dir_for_agent(&mstore, "collide-agent").unwrap();
    let as_str = dir.to_string_lossy().to_string();
    assert!(
        !as_str.contains("somebody-elses-dir"),
        "must not resolve another definition's memory dir: {as_str}",
    );
    assert!(
        as_str.contains("collide-agent"),
        "expected the derived default for this agent: {as_str}",
    );

    match prev {
        Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
        None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
    }
}

// ── Memory directory provenance (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2, M1)

fn segment_start(uid: &str, config_dir: Option<&str>, cwd: &str, at: i64) -> crate::backend::continuity_segments::Start {
    crate::backend::continuity_segments::Start {
        segment_id: String::new(),
        agent_uid: uid.into(),
        definition_id: Some(uid.into()),
        provider: "claude".into(),
        config_dir: config_dir.map(Into::into),
        provider_session_id: None,
        cwd: cwd.into(),
        channel: "test".into(),
        agentmux_version: "test".into(),
        block_id: "block-1".into(),
        zone: None,
        byte_start: None,
        started_at_ms: at,
        continuity_rung: crate::backend::continuity_segments::rung_for_spawn(false, false),
        predecessor_segment_id: None,
        lease_epoch: None,
        identity_key: None,
        forked_from: None,
    }
}

#[test]
fn the_latest_spawn_decides_the_memory_directory() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mstore = Store::open_in_memory().unwrap();
    let mut def = agent_def("agent-seg", "/work/row-says");
    mstore.agent_def_insert(&mut def).unwrap();
    let fs = crate::backend::storage::filestore::FileStore::open_in_memory().unwrap();
    crate::backend::continuity_segments::record_start(&fs, segment_start("agent-seg", Some("/cfg/old"), "/work/old", 1_000)).unwrap();
    crate::backend::continuity_segments::record_start(&fs, segment_start("agent-seg", Some("/cfg/new"), "/work/new", 2_000)).unwrap();

    let r = resolve_memory_dir_with(&mstore, &def, Some(&fs)).unwrap();
    assert_eq!(r.provenance, MemoryDirProvenance::Spawn);
    assert_eq!(r.path, memory_dir_for_cwd("/cfg/new", "/work/new"), "the newest spawn wins over the row");
}

#[test]
fn a_blank_working_directory_with_a_spawn_on_record_is_verified() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mstore = Store::open_in_memory().unwrap();
    let mut def = agent_def("agent-seg-blank", "");
    mstore.agent_def_insert(&mut def).unwrap();
    let fs = crate::backend::storage::filestore::FileStore::open_in_memory().unwrap();

    let unverified = resolve_memory_dir_with(&mstore, &def, Some(&fs)).unwrap();
    assert_eq!(unverified.provenance, MemoryDirProvenance::Unverified, "no spawn yet: a guess");

    crate::backend::continuity_segments::record_start(&fs, segment_start("agent-seg-blank", Some("/cfg"), "/home/u/.agentmux/agents/x", 1_000)).unwrap();
    let verified = resolve_memory_dir_with(&mstore, &def, Some(&fs)).unwrap();
    assert_eq!(verified.provenance, MemoryDirProvenance::Spawn);
}

#[test]
fn an_explicit_working_directory_is_verified_without_a_spawn() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mstore = Store::open_in_memory().unwrap();
    let mut def = agent_def("agent-wd", "/work/explicit");
    mstore.agent_def_insert(&mut def).unwrap();
    let r = resolve_memory_dir_with(&mstore, &def, None).unwrap();
    assert_eq!(r.provenance, MemoryDirProvenance::WorkingDirectory);
}

#[test]
fn a_directory_two_agents_resolve_to_is_not_attributed_to_either() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mstore = Store::open_in_memory().unwrap();
    for id in ["agent-share-a", "agent-share-b"] {
        let mut def = agent_def(id, "/work/shared");
        mstore.agent_def_insert(&mut def).unwrap();
    }
    let mut solo = agent_def("agent-solo", "/work/solo");
    mstore.agent_def_insert(&mut solo).unwrap();

    let targets = list_memory_targets_with(&mstore, None);
    let ids: Vec<&str> = targets.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["agent-solo"], "{targets:?}");
}

#[test]
fn a_spawn_recorded_with_an_unexpanded_home_names_claudes_real_folder() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mstore = Store::open_in_memory().unwrap();
    let mut def = agent_def("agent-tilde", "");
    mstore.agent_def_insert(&mut def).unwrap();
    let fs = crate::backend::storage::filestore::FileStore::open_in_memory().unwrap();
    crate::backend::continuity_segments::record_start(&fs, segment_start("agent-tilde", Some("/cfg"), "~/.agentmux/agents/tilde", 1_000)).unwrap();

    let r = resolve_memory_dir_with(&mstore, &def, Some(&fs)).unwrap();
    let expanded = expand_home_dir_safe("~/.agentmux/agents/tilde");
    let folder = crate::backend::claude_layout::project_dir_name(&expanded.to_string_lossy());
    assert_eq!(r.path, std::path::PathBuf::from("/cfg").join("projects").join(&folder).join("memory"));
    assert!(!folder.starts_with("--"), "the home dir is part of the folder name: {folder}");
}
