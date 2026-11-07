//! `skill.catalog.app_file` and `skill.catalog.app_script`: read a script, approve it, revoke it.

use super::super::*;
use super::unused_name;
use crate::backend::rpc_types::*;
use serde_json::{json, Value};

async fn call(state: &AppState, command: &str, data: Value) -> Result<Value, String> {
    let (engine, mut rx) = WshRpcEngine::new();
    register(&engine, state);
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: "req-1".to_string(), data: Some(data), ..Default::default() });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    if resp.error.is_empty() { Ok(resp.data.expect("result data")) } else { Err(resp.error) }
}

const SCRIPT: &str = "print('hi')\n";

fn sha(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Import a computer-use folder with one listed script, through the real preview and commit.
async fn import_with_script(state: &AppState) -> String {
    let src = tempfile::tempdir().unwrap();
    let dir = src.path().join("acmeappr-wordt");
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::write(dir.join("SKILL.md"), "---\nname: acmeappr-wordt\ndescription: Use when driving Word.\n---\n\n# Word\n").unwrap();
    std::fs::write(dir.join("test.json"), "{\"skill\":\"acmeappr.wordt\",\"task\":\"t\",\"steps\":[],\"verify\":[\"a\"]}\n").unwrap();
    std::fs::write(dir.join("scripts/run.py"), SCRIPT).unwrap();
    std::fs::write(
        dir.join("computer-use.json"),
        format!(
            r#"{{"schema":"computer-use-skill/v1","publisher":"acmeappr","id":"wordt","name":"Word","version":"1.0.0",
"changelog":[{{"version":"1.0.0","date":"2026-11-05","by":"x","change":"first"}}],
"app":{{"vendor":"Acme","executables":{{"windows":["WINWORD.EXE"]}},"titleHints":["Word"],"versions":{{"supported":">=16","tested":["16.0"]}}}},
"platforms":["windows"],"ladder":[{{"kind":"api","note":"COM"}}],"mcp":null,
"scripts":[{{"path":"scripts/run.py","sha256":"{}","purpose":"open a file","needs":["python 3"]}}]}}"#,
            sha(SCRIPT)
        ),
    )
    .unwrap();
    let source = json!({"kind": "folder", "location": src.path().to_string_lossy(), "git_ref": "", "item_key": ""});
    let p = call(state, COMMAND_SKILL_CATALOG_IMPORT_PREVIEW, json!({"source": source})).await.unwrap();
    let item = &p["items"][0];
    assert_eq!(item["app"]["scripts"][0]["purpose"], "open a file", "the preview shows what the script is for: {p}");
    assert_eq!(item["app"]["signature"]["state"], "unsigned");
    let commit = json!({"source": source, "items": [{"key": item["key"], "name": item["name"], "description": item["description"], "content": item["content"], "existing_id": "", "app_hash": item["app_hash"]}]});
    let saved = call(state, COMMAND_SKILL_CATALOG_IMPORT_COMMIT, commit).await.unwrap();
    saved["saved"][0].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_script_is_read_then_approved_and_revoked_and_nothing_else_can_be() {
    let state = crate::server::tests::test_state();
    let id = import_with_script(&state).await;

    let f = call(&state, COMMAND_SKILL_CATALOG_APP_FILE, json!({"skill_id": id, "path": "scripts/run.py"})).await.unwrap();
    assert_eq!(f["text"], SCRIPT);
    assert!(call(&state, COMMAND_SKILL_CATALOG_APP_FILE, json!({"skill_id": id, "path": "nope.md"})).await.is_err());

    let approved = |state: &AppState| state.identity_store.skill_files(&id).unwrap().into_iter().find(|f| f.path == "scripts/run.py").unwrap().approved;
    assert!(!approved(&state), "an imported script starts unapproved");
    let sha = crate::backend::computer_use_skill::sha256_hex(SCRIPT.as_bytes());
    // Only the text that was read can be approved.
    let e = call(&state, COMMAND_SKILL_CATALOG_APP_SCRIPT, json!({"skill_id": id, "path": "scripts/run.py", "approved": true, "sha256": "0".repeat(64)})).await.unwrap_err();
    assert!(e.contains("not the text you reviewed"), "{e}");
    assert!(!approved(&state));
    call(&state, COMMAND_SKILL_CATALOG_APP_SCRIPT, json!({"skill_id": id, "path": "scripts/run.py", "approved": true, "sha256": sha})).await.unwrap();
    assert!(approved(&state));
    call(&state, COMMAND_SKILL_CATALOG_APP_SCRIPT, json!({"skill_id": id, "path": "scripts/run.py", "approved": false, "sha256": ""})).await.unwrap();
    assert!(!approved(&state));

    let e = call(&state, COMMAND_SKILL_CATALOG_APP_SCRIPT, json!({"skill_id": id, "path": "test.json", "approved": true, "sha256": ""})).await.unwrap_err();
    assert!(e.contains("not a script"), "{e}");
    let plain = call(&state, COMMAND_SKILL_CATALOG_UPSERT, json!({"name": "plain-app-rpc-t", "skill_type": "agent-skill"})).await.unwrap();
    let e = call(&state, COMMAND_SKILL_CATALOG_APP_SCRIPT, json!({"skill_id": plain["id"], "path": "x", "approved": true, "sha256": ""})).await.unwrap_err();
    assert!(e.contains("not a computer-use skill"), "{e}");
}

#[tokio::test]
async fn a_skill_file_is_previewed_like_a_folder_with_its_scripts_and_signature() {
    let state = crate::server::tests::test_state();
    let id = import_with_script(&state).await;
    let exported = crate::backend::skill_file::export(&state.identity_store, &id).unwrap();
    assert_eq!(exported.filename, "acmeappr-wordt-1.0.0.skill");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(&exported.filename);
    std::fs::write(&file, &exported.bytes).unwrap();

    let source = json!({"kind": "file", "location": file.to_string_lossy(), "git_ref": "", "item_key": ""});
    let p = call(&state, COMMAND_SKILL_CATALOG_IMPORT_PREVIEW, json!({"source": source})).await.unwrap();
    let item = &p["items"][0];
    assert_eq!(item["name"], "acmeappr-wordt");
    assert_eq!(item["status"], "same", "the exported skill is what the catalog already holds: {p}");
    assert_eq!(item["app"]["scripts"][0]["purpose"], "open a file");

    let missing = json!({"kind": "file", "location": dir.path().join("nope.skill").to_string_lossy(), "git_ref": "", "item_key": ""});
    assert!(call(&state, COMMAND_SKILL_CATALOG_IMPORT_PREVIEW, json!({"source": missing})).await.is_err());
}

#[test]
fn an_export_never_overwrites_a_file_that_is_there() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(unused_name(dir.path(), "a-b-1.0.0.skill"), dir.path().join("a-b-1.0.0.skill"));
    std::fs::write(dir.path().join("a-b-1.0.0.skill"), b"x").unwrap();
    assert_eq!(unused_name(dir.path(), "a-b-1.0.0.skill"), dir.path().join("a-b-1.0.0 (2).skill"));
    std::fs::write(dir.path().join("a-b-1.0.0 (2).skill"), b"x").unwrap();
    assert_eq!(unused_name(dir.path(), "a-b-1.0.0.skill"), dir.path().join("a-b-1.0.0 (3).skill"));
}
