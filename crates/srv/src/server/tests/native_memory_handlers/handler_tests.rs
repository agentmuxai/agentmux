// Direct calls to the named native-memory handlers: no engine, no RPC
// framing. The engine-level paths (broker events, shared-channel writes,
// conditional saves) are covered in `tests.rs`.

use super::*;
use crate::backend::storage::store::Store;
use crate::backend::storage::AgentContent;

/// The state under test plus the temp files that keep it alive. The agent's
/// memory directory is a tempdir (via `CLAUDE_CONFIG_DIR`), never the real home.
struct Fixture {
    state: AppState,
    _config: tempfile::TempDir,
    _id_db: tempfile::NamedTempFile,
}

const AGENT: &str = "agent-direct-1";

async fn fixture() -> Fixture {
    let config = tempfile::tempdir().unwrap();
    let id_db = tempfile::NamedTempFile::new().unwrap();
    let mstore = Arc::new(Store::open_in_memory().unwrap());
    let mut def = super::tests::agent_def(AGENT, "/work/direct-1");
    mstore.agent_def_insert(&mut def).unwrap();
    mstore
        .agent_content_set(&AgentContent {
            agent_id: AGENT.to_string(),
            content_type: "env".to_string(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", config.path().display()),
            updated_at: 0,
        })
        .unwrap();

    let mut state = crate::server::tests::test_state();
    state.mstore = mstore;
    state.id_store = Arc::new(Store::open_shared(id_db.path()).unwrap());
    Fixture { state, _config: config, _id_db: id_db }
}

fn write_req(filename: &str, content: &str) -> CommandNativeMemoryWriteFileData {
    serde_json::from_value(serde_json::json!({
        "agent_id": AGENT, "filename": filename, "content": content,
    }))
    .unwrap()
}

fn read_req(filename: &str) -> CommandNativeMemoryReadFileData {
    serde_json::from_value(serde_json::json!({ "agent_id": AGENT, "filename": filename })).unwrap()
}

fn history_req(filename: &str) -> CommandNativeMemoryHistoryData {
    serde_json::from_value(serde_json::json!({ "agent_id": AGENT, "filename": filename })).unwrap()
}

#[tokio::test]
async fn a_written_file_reads_back_and_is_listed_with_the_index_first() {
    let f = fixture().await;
    native_memory_write_file(&f.state, write_req("notes.md", "a note")).unwrap();
    native_memory_write_file(&f.state, write_req("MEMORY.md", "# index")).unwrap();

    let read = native_memory_read_file(&f.state, read_req("notes.md")).unwrap();
    assert_eq!(read.content, "a note");

    let list = native_memory_list(
        &f.state,
        serde_json::from_value(serde_json::json!({ "agent_id": AGENT })).unwrap(),
    )
    .unwrap();
    let names: Vec<&str> = list.files.iter().map(|m| m.filename.as_str()).collect();
    assert_eq!(names.first(), Some(&"MEMORY.md"), "the index is listed first: {names:?}");
    assert!(names.contains(&"notes.md"), "{names:?}");
    assert!(list.files[0].is_index);
}

#[tokio::test]
async fn write_refuses_an_invalid_filename() {
    let f = fixture().await;
    let err = native_memory_write_file(&f.state, write_req("../escape.md", "x")).unwrap_err();
    assert!(err.starts_with("agent:memory:write_file:"), "{err}");
}

#[tokio::test]
async fn write_refuses_content_over_the_size_limit() {
    let f = fixture().await;
    let big = "x".repeat(10 * 1024 * 1024 + 1);
    let err = native_memory_write_file(&f.state, write_req("big.md", &big)).unwrap_err();
    assert!(err.contains("content too large"), "{err}");
}

#[tokio::test]
async fn write_for_an_unknown_agent_is_an_error() {
    let f = fixture().await;
    let mut req = write_req("notes.md", "x");
    req.agent_id = "no-such-agent".to_string();
    let err = native_memory_write_file(&f.state, req).unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

#[tokio::test]
async fn history_records_each_write_and_revert_restores_the_first() {
    let f = fixture().await;
    native_memory_write_file(&f.state, write_req("MEMORY.md", "first")).unwrap();
    native_memory_write_file(&f.state, write_req("MEMORY.md", "second")).unwrap();

    let history = native_memory_history(&f.state, history_req("MEMORY.md")).unwrap();
    assert_eq!(history.versions.len(), 2, "one version per write");
    let first = history
        .versions
        .iter()
        .find(|v| v.parent_version_id.is_none())
        .expect("the first write has no parent");

    let reverted = native_memory_revert(
        &f.state,
        serde_json::from_value(serde_json::json!({
            "agent_id": AGENT, "filename": "MEMORY.md", "target_version_id": first.id,
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(reverted.version.content_hash, first.content_hash);
    assert_eq!(native_memory_read_file(&f.state, read_req("MEMORY.md")).unwrap().content, "first");

    let after = native_memory_history(&f.state, history_req("MEMORY.md")).unwrap();
    assert_eq!(after.versions.len(), 3, "a revert is itself a new version");
}

#[tokio::test]
async fn diff_shows_the_change_between_two_versions() {
    let f = fixture().await;
    native_memory_write_file(&f.state, write_req("MEMORY.md", "alpha\n")).unwrap();
    native_memory_write_file(&f.state, write_req("MEMORY.md", "beta\n")).unwrap();
    let versions = native_memory_history(&f.state, history_req("MEMORY.md")).unwrap().versions;
    let old = versions.iter().find(|v| v.parent_version_id.is_none()).unwrap();
    let new = versions.iter().find(|v| v.parent_version_id.is_some()).unwrap();

    let diff = native_memory_diff(
        &f.state,
        serde_json::from_value(serde_json::json!({
            "agent_id": AGENT, "from_version_id": old.id, "to_version_id": new.id,
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(diff.diff.contains("alpha") && diff.diff.contains("beta"), "{}", diff.diff);
}

#[test]
fn write_memory_file_atomically_replaces_the_file_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    write_memory_file_atomically(dir.path(), "a.md", "one", "test").unwrap();
    write_memory_file_atomically(dir.path(), "a.md", "two", "test").unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join("a.md")).unwrap(), "two");
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1, "only the destination remains");
}

#[test]
fn write_memory_file_atomically_reports_a_missing_directory() {
    let dir = tempfile::tempdir().unwrap();
    let err = write_memory_file_atomically(&dir.path().join("absent"), "a.md", "x", "test").unwrap_err();
    assert!(err.starts_with("test: write tmp:"), "{err}");
}
