// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/skill`: the token is required, only catalog skills can be changed, an edit
//! keeps what it leaves out, and a new skill reaches the agents created after it. The per-site
//! `m4.actor_*` counters move with a token, so these tests take the lock the counter test takes.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::backend::storage::Skill;
use crate::server::routes::build_router;
use crate::server::tests::{test_state, token_for, M4_ACTOR_COUNTERS};
use crate::server::AppState;

const AGENT: &str = "sk-alpha";

async fn call(state: &AppState, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/agent/skill")
        .header("X-AuthKey", "test-secret-key")
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let resp = build_router(state.clone()).oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn error_of(v: &Value) -> &str {
    v["error"].as_str().unwrap_or("")
}

fn skill(id: &str, name: &str, is_global: bool) -> Skill {
    Skill {
        id: id.to_string(),
        name: name.to_string(),
        trigger: String::new(),
        skill_type: "agent-skill".to_string(),
        description: "when to use".to_string(),
        content: "the text".to_string(),
        is_global,
        created_at: 1,
        updated_at: 1,
    }
}

#[tokio::test]
async fn a_request_without_the_agents_token_is_refused() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let (_, v) = call(&state, None, json!({ "action": "list", "agent_id": AGENT })).await;
    assert!(error_of(&v).starts_with("FORBIDDEN"), "{v}");
}

#[tokio::test]
async fn an_edit_keeps_what_it_leaves_out_and_a_new_skill_reaches_later_agents() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let token = token_for(&state, AGENT);

    let (_, made) = call(
        &state,
        Some(&token),
        json!({ "action": "upsert", "agent_id": AGENT, "name": "tidy", "description": "when tidy", "content": "v1" }),
    )
    .await;
    let id = made["id"].as_str().expect(&made.to_string()).to_string();

    let (_, listed) = call(&state, Some(&token), json!({ "action": "list", "agent_id": AGENT })).await;
    let row = listed.as_array().unwrap().iter().find(|r| r["id"] == id.as_str()).expect("listed");
    assert_eq!(row["all_agents"], false);
    assert_eq!(row["new_agents"], true);
    assert!(row.get("content").is_none(), "list carries no skill text");

    call(&state, Some(&token), json!({ "action": "upsert", "agent_id": AGENT, "skill_id": id, "content": "v2" })).await;
    let (_, got) = call(&state, Some(&token), json!({ "action": "get", "agent_id": AGENT, "skill_id": id })).await;
    assert_eq!(got["content"], "v2");
    assert_eq!(got["name"], "tidy");
    assert_eq!(got["description"], "when tidy");
}

#[tokio::test]
async fn a_private_skill_cannot_be_read_changed_or_given_away_here() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let token = token_for(&state, AGENT);
    let uid = format!("uid-{AGENT}");
    state.mstore.skill_upsert_unique(&state.identity_store, &uid, &skill("sk-private", "mine", false), true).unwrap();

    for action in ["get", "delete", "bind", "unbind", "reach"] {
        let (_, v) = call(&state, Some(&token), json!({ "action": action, "agent_id": AGENT, "skill_id": "sk-private" })).await;
        assert!(error_of(&v).starts_with("FORBIDDEN"), "{action}: {v}");
    }
    let (_, v) = call(
        &state,
        Some(&token),
        json!({ "action": "upsert", "agent_id": AGENT, "skill_id": "sk-private", "content": "x" }),
    )
    .await;
    assert!(error_of(&v).starts_with("FORBIDDEN"), "{v}");
    assert_eq!(state.identity_store.skill_get("sk-private").unwrap().unwrap().content, "the text");
}

#[tokio::test]
async fn bind_with_no_target_binds_the_caller_and_reach_updates_only_what_is_given() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let token = token_for(&state, AGENT);
    let (_, made) = call(&state, Some(&token), json!({ "action": "upsert", "agent_id": AGENT, "name": "bound" })).await;
    let id = made["id"].as_str().unwrap().to_string();

    call(&state, Some(&token), json!({ "action": "bind", "agent_id": AGENT, "skill_id": id })).await;
    let bound = state.mstore.effective_skills(&state.identity_store, &format!("uid-{AGENT}"));
    assert!(bound.iter().any(|s| s.name == "bound"));

    let (_, v) = call(&state, Some(&token), json!({ "action": "reach", "agent_id": AGENT, "skill_id": id, "all_agents": true })).await;
    assert_eq!(v["all_agents"], true);
    assert_eq!(v["new_agents"], true);
    let (_, v) = call(&state, Some(&token), json!({ "action": "reach", "agent_id": AGENT, "skill_id": id, "all_agents": false })).await;
    assert_eq!(v["all_agents"], false);
    assert_eq!(v["new_agents"], true);
}

// ── computer-use skills: files beside the text, and test records ──────────────────────────────

const APP_MANIFEST: &str = r#"{"schema":"computer-use-skill/v1","publisher":"acme","id":"word","name":"Word","version":"1.0.0",
"changelog":[{"version":"1.0.0"}],"app":{"vendor":"Acme","executables":{"windows":["WINWORD.EXE"]},
"versions":{"supported":">=16","tested":[]}},"platforms":["windows"],"ladder":[{"kind":"api","note":"COM"}],
"references":[{"path":"references/fader.png","label":"fader","shows":"cap","appVersion":"16.0","platform":"windows","theme":"dark","scale":100,"capturedBy":"AgentMux test machine","date":"2026-11-06","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}]}"#;

/// A computer-use skill in the catalog: a reference file, an unapproved script, an approved script and a crop.
fn install_app_skill(state: &AppState) -> String {
    use crate::backend::storage::skill_app::SkillFileInput;
    let s = skill("sk-app", "acme-word", true);
    state.identity_store.skill_upsert_unique_global(&s).unwrap();
    let info = crate::backend::computer_use_skill::info_from_manifest(APP_MANIFEST).unwrap();
    let file = |path: &str, content: &str, script: bool| SkillFileInput { path: path.into(), content: content.into(), image: None, script };
    let files = [
        file("controls.md", "# controls", false),
        file("scripts/run.py", "print(1)", true),
        SkillFileInput { path: "references/fader.png".into(), content: String::new(), image: Some(vec![0x89, b'P', b'N', b'G', 1, 2]), script: false },
    ];
    state.identity_store.skill_app_set("sk-app", &info, APP_MANIFEST, "hash", &files, 1).unwrap();
    "sk-app".to_string()
}

#[tokio::test]
async fn a_computer_use_skill_lists_its_app_and_get_lists_its_files() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let token = token_for(&state, AGENT);
    let id = install_app_skill(&state);

    let (_, listed) = call(&state, Some(&token), json!({ "action": "list", "agent_id": AGENT })).await;
    let row = listed.as_array().unwrap().iter().find(|r| r["id"] == id.as_str()).expect("listed");
    assert_eq!((row["app"]["vendor"].as_str(), row["app"]["version"].as_str()), (Some("Acme"), Some("1.0.0")));

    let (_, got) = call(&state, Some(&token), json!({ "action": "get", "agent_id": AGENT, "skill_id": id })).await;
    let paths: Vec<&str> = got["app"]["files"].as_array().unwrap().iter().filter_map(|f| f["path"].as_str()).collect();
    assert_eq!(paths, vec!["controls.md", "references/fader.png", "scripts/run.py"]);
    assert_eq!(got["app"]["references"][0]["appVersion"], "16.0");
    assert!(got["note"].as_str().unwrap().contains("get and file"));
}

#[tokio::test]
async fn get_with_file_reads_text_flags_an_unapproved_script_and_returns_a_crop_with_its_tags() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let token = token_for(&state, AGENT);
    let id = install_app_skill(&state);
    let get = |file: &str| json!({ "action": "get", "agent_id": AGENT, "skill_id": id, "file": file });

    let (_, text) = call(&state, Some(&token), get("controls.md")).await;
    assert_eq!((text["text"].as_str(), text.get("script")), (Some("# controls"), None));

    let (_, script) = call(&state, Some(&token), get("scripts/run.py")).await;
    assert_eq!((script["script"].clone(), script["approved"].clone()), (json!(true), json!(false)));
    assert!(script["note"].as_str().unwrap().contains("not been approved"));

    let (_, crop) = call(&state, Some(&token), get("references/fader.png")).await;
    assert_eq!(crop["mime"], "image/png");
    assert_eq!(crop["tags"]["label"], "fader");
    assert_eq!(crop["tags"]["appVersion"], "16.0");
    assert!(!crop["image_base64"].as_str().unwrap().is_empty());

    let (_, missing) = call(&state, Some(&token), get("nope.md")).await;
    assert!(error_of(&missing).contains("it has: controls.md, references/fader.png, scripts/run.py"), "{missing}");

    // A plain skill has nothing beside its text.
    state.identity_store.skill_upsert_unique_global(&skill("sk-plain", "plain", true)).unwrap();
    let (_, plain) = call(&state, Some(&token), json!({ "action": "get", "agent_id": AGENT, "skill_id": "sk-plain", "file": "x.md" })).await;
    assert!(error_of(&plain).contains("no files beside its text"), "{plain}");
}

#[tokio::test]
async fn a_test_record_is_checked_kept_once_and_only_for_the_installed_version() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let token = token_for(&state, AGENT);
    let id = install_app_skill(&state);
    let rec = |extra: Value| {
        let mut v = json!({ "action": "record", "agent_id": AGENT, "skill_id": id, "skill_version": "1.0.0", "app_version": "16.0", "platform": "windows", "result": "pass" });
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    };

    let (_, ok) = call(&state, Some(&token), rec(json!({ "route": "api", "date": "2026-11-06", "evidence_sha256": "AB".repeat(32) }))).await;
    assert_eq!(ok["recorded"], true, "{ok}");
    let (_, again) = call(&state, Some(&token), rec(json!({ "route": "api", "date": "2026-11-06", "evidence_sha256": "ab".repeat(32) }))).await;
    assert_eq!(again["recorded"], false, "the same record twice is kept once");
    let stored = state.identity_store.skill_app_get(&id).unwrap().unwrap().verified;
    assert_eq!(stored.len(), 1);
    assert_eq!((stored[0].runner.as_str(), stored[0].evidence_sha256.as_str()), ("agent:sk-alpha", "ab".repeat(32).as_str()));

    for (extra, want) in [
        (json!({ "skill_version": "0.9.0" }), "installed version"),
        (json!({ "platform": "beos" }), "platform"),
        (json!({ "result": "maybe" }), "pass or fail"),
        (json!({ "route": "telepathy" }), "route"),
        (json!({ "evidence_sha256": "not-a-hash" }), "64 hex"),
        (json!({ "date": "yesterday" }), "YYYY-MM-DD"),
        (json!({ "app_version": "" }), "app_version"),
    ] {
        let (_, v) = call(&state, Some(&token), rec(extra.clone())).await;
        assert!(error_of(&v).contains(want), "{extra}: {v}");
    }
    // Nothing else was added, and a plain skill takes no record.
    assert_eq!(state.identity_store.skill_app_get(&id).unwrap().unwrap().verified.len(), 1);
    state.identity_store.skill_upsert_unique_global(&skill("sk-plain", "plain", true)).unwrap();
    let (_, plain) = call(&state, Some(&token), json!({ "action": "record", "agent_id": AGENT, "skill_id": "sk-plain", "skill_version": "1.0.0" })).await;
    assert!(error_of(&plain).contains("not a computer-use skill"), "{plain}");
}

#[tokio::test]
async fn deleting_a_computer_use_skill_drops_its_files_and_records() {
    let _counters = M4_ACTOR_COUNTERS.lock().await;
    let state = test_state();
    let token = token_for(&state, AGENT);
    let id = install_app_skill(&state);
    call(&state, Some(&token), json!({ "action": "delete", "agent_id": AGENT, "skill_id": id })).await;
    assert!(state.identity_store.skill_app_get(&id).unwrap().is_none());
    assert!(state.identity_store.skill_files(&id).unwrap().is_empty());
}
