//! `skill.catalog.import_preview` / `import_commit`: read a source, show what it would change,
//! and write only what was approved.

use super::*;
use serde_json::{json, Value};

async fn call(state: &AppState, command: &str, data: Value) -> Result<Value, String> {
    let (engine, mut rx) = WshRpcEngine::new();
    register(&engine, state);
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: "req-1".to_string(), data: Some(data), ..Default::default() });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    if resp.error.is_empty() { Ok(resp.data.expect("result data")) } else { Err(resp.error) }
}

fn write_skill(dir: &std::path::Path, rel: &str, name: &str, body: &str) {
    let p = dir.join(rel).join("SKILL.md");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, format!("---\nname: {name}\ndescription: Use for {name}\n---\n\n{body}")).unwrap();
}

fn folder(dir: &std::path::Path) -> Value {
    json!({"kind": "folder", "location": dir.to_string_lossy(), "git_ref": "", "item_key": ""})
}

fn commit_all(preview: &Value, source: &Value) -> Value {
    let items: Vec<Value> = preview["items"].as_array().unwrap().iter().map(|i| json!({
        "key": i["key"], "name": i["name"], "description": i["description"], "content": i["content"], "existing_id": i["existing_id"]
    })).collect();
    json!({"source": source, "items": items})
}

#[tokio::test]
async fn an_import_previews_commits_unbound_and_shows_a_diff_when_the_source_changes() {
    let state = crate::server::tests::test_state();
    let src = tempfile::tempdir().unwrap();
    write_skill(src.path(), "alpha", "alpha-import-t", "one");
    write_skill(src.path(), "beta", "beta-import-t", "two");
    let source = folder(src.path());

    // Preview writes nothing.
    let p = call(&state, COMMAND_SKILL_CATALOG_IMPORT_PREVIEW, json!({"source": source})).await.unwrap();
    let statuses: Vec<&str> = p["items"].as_array().unwrap().iter().map(|i| i["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["new", "new"]);
    let listed = call(&state, COMMAND_SKILL_CATALOG_LIST, json!({})).await.unwrap();
    assert!(!listed.to_string().contains("alpha-import-t"), "a preview must not write");

    // Commit: global, agent-skill, bound to nobody, outside "new agents get it", source recorded.
    let saved = call(&state, COMMAND_SKILL_CATALOG_IMPORT_COMMIT, commit_all(&p, &source)).await.unwrap();
    assert_eq!(saved["saved"].as_array().unwrap().len(), 2);
    let listed = call(&state, COMMAND_SKILL_CATALOG_LIST, json!({})).await.unwrap();
    let alpha = listed.as_array().unwrap().iter().find(|s| s["name"] == "alpha-import-t").unwrap().clone();
    assert_eq!(alpha["skill_type"], "agent-skill");
    assert_eq!((alpha["bound_count"].as_i64(), alpha["all_agents"].as_bool(), alpha["new_agents"].as_bool()), (Some(0), Some(false), Some(false)));
    assert_eq!(alpha["source"]["kind"], "folder");
    assert_eq!(alpha["source"]["item_key"], "alpha");

    // Unchanged source: "same". Changed source: "changed", with the text now in the catalog beside it.
    let p2 = call(&state, COMMAND_SKILL_CATALOG_IMPORT_PREVIEW, json!({"source": source})).await.unwrap();
    assert!(p2["items"].as_array().unwrap().iter().all(|i| i["status"] == "same"));
    write_skill(src.path(), "alpha", "alpha-import-t", "one, revised");
    let alpha_id = alpha["id"].as_str().unwrap();
    let p3 = call(&state, COMMAND_SKILL_CATALOG_IMPORT_PREVIEW, json!({"skill_id": alpha_id})).await.unwrap();
    let items = p3["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "an update checks only its own skill");
    assert_eq!(items[0]["status"], "changed");
    assert!(items[0]["content"].as_str().unwrap().contains("revised"));
    assert!(items[0]["existing_content"].as_str().unwrap().contains("one") && !items[0]["existing_content"].as_str().unwrap().contains("revised"));
    // Not applied until committed.
    let still = call(&state, COMMAND_SKILL_CATALOG_LIST, json!({})).await.unwrap();
    assert!(!still.to_string().contains("revised"));

    // Approving it updates the same skill, keeping its id.
    let saved = call(&state, COMMAND_SKILL_CATALOG_IMPORT_COMMIT, commit_all(&p3, &p3["source"])).await.unwrap();
    assert_eq!(saved["saved"][0], alpha_id);
    let after = call(&state, COMMAND_SKILL_CATALOG_LIST, json!({})).await.unwrap();
    let alpha = after.as_array().unwrap().iter().find(|s| s["id"] == alpha_id).unwrap().clone();
    assert!(alpha["content"].as_str().unwrap().contains("revised"));
}

#[tokio::test]
async fn bad_sources_and_hand_written_skills_are_refused_with_a_reason() {
    let state = crate::server::tests::test_state();
    for source in [
        json!({"kind": "url", "location": "http://example.com/SKILL.md"}),
        json!({"kind": "git", "location": "file:///etc"}),
        json!({"kind": "git", "location": "https://example.com/r.git", "git_ref": "--upload-pack=x"}),
        json!({"kind": "ftp", "location": "x"}),
    ] {
        let e = call(&state, COMMAND_SKILL_CATALOG_IMPORT_PREVIEW, json!({"source": source})).await.unwrap_err();
        assert!(e.contains("skill.catalog.import_preview"), "{e}");
    }
    let made = call(&state, COMMAND_SKILL_CATALOG_UPSERT, json!({"name": "by-hand-import-t", "skill_type": "agent-skill"})).await.unwrap();
    let e = call(&state, COMMAND_SKILL_CATALOG_IMPORT_PREVIEW, json!({"skill_id": made["id"]})).await.unwrap_err();
    assert!(e.contains("not imported"), "{e}");
    assert!(call(&state, COMMAND_SKILL_CATALOG_IMPORT_PREVIEW, json!({})).await.is_err());
}

#[tokio::test]
async fn a_bad_item_in_a_batch_writes_nothing() {
    let state = crate::server::tests::test_state();
    let source = json!({"kind": "folder", "location": "x", "git_ref": "", "item_key": ""});
    let item = |key: &str, name: &str| json!({"key": key, "name": name, "description": "d", "content": "c", "existing_id": ""});
    for items in [
        vec![item("a", "batch-t-one"), item("b", "batch-t-one")], // two with one name
        vec![item("a", "batch-t-two"), item("b", &"n".repeat(201))], // a name over the limit
    ] {
        let e = call(&state, COMMAND_SKILL_CATALOG_IMPORT_COMMIT, json!({"source": source, "items": items})).await.unwrap_err();
        assert!(e.contains("skill.catalog.import_commit"), "{e}");
        let listed = call(&state, COMMAND_SKILL_CATALOG_LIST, json!({})).await.unwrap().to_string();
        assert!(!listed.contains("batch-t-"), "the earlier item must not have been written: {listed}");
    }
}
