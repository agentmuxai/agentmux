// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

const HOST: (&str, &str) = ("X-Host-Token", "host-ipc-token");

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

fn post(uri: &str, headers: &[(&str, &str)], body: Value) -> Request<Body> {
    let mut b = Request::builder().uri(uri).method("POST").header("X-AuthKey", "test-secret-key").header("Content-Type", "application/json");
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).header("X-AuthKey", "test-secret-key").body(Body::empty()).unwrap()
}

async fn app() -> axum::Router {
    crate::backend::vault::seal::use_test_key();
    let state = crate::server::tests::test_state();
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
    crate::server::build_router(state)
}

fn new_ssn() -> Value {
    json!({ "op": "create", "kind": "ssn", "label": "Alex", "fields": { "owner": "Alex" }, "parts": { "value": "123-45-6789" } })
}

// §5.4: the instance key every agent holds can't write or read the Vault
// through the pane's route; only the host's token can.
#[tokio::test]
async fn only_the_host_reaches_the_panes_route() {
    let app = app().await;
    for op in [new_ssn(), json!({ "op": "list" }), json!({ "op": "delete", "id": "ssn-alex" })] {
        let (s, body) = send(&app, post("/api/v1/host/vault", &[], op.clone())).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{op}");
        assert!(body.to_string().contains("only the AgentMux host"));
        let (s, _) = send(&app, post("/api/v1/host/vault", &[("X-Host-Token", "a-guess")], op)).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn the_pane_saves_and_lists_and_agents_see_only_names() {
    let app = app().await;
    let (s, body) = send(&app, post("/api/v1/host/vault", &[HOST], new_ssn())).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let id = body["data"]["id"].as_str().unwrap().to_string();

    let (_, body) = send(&app, post("/api/v1/host/vault", &[HOST], json!({ "op": "list" }))).await;
    let entry = body["data"]["entries"].as_array().unwrap().iter().find(|e| e["id"] == id.as_str()).unwrap().clone();
    assert_eq!(entry["tail"], "6789");
    assert!(!body.to_string().contains("123-45"), "the pane never gets a whole value: {body}");

    let (s, body) = send(&app, get("/api/v1/vault")).await;
    assert_eq!(s, StatusCode::OK);
    let text = body.to_string();
    assert!(text.contains(&id) && text.contains("\"label\":\"Alex\""), "{text}");
    for leak in ["6789", "tail", "owner", "fields", "sealed"] {
        assert!(!text.contains(leak), "an agent's list has no {leak}: {text}");
    }

    let (s, _) = send(&app, post("/api/v1/host/vault", &[HOST], json!({ "op": "delete", "id": id }))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, body) = send(&app, get("/api/v1/vault")).await;
    assert!(!body.to_string().contains(&id));
}

#[tokio::test]
async fn an_agents_request_is_answered_with_the_new_entrys_id_only() {
    let app = app().await;
    let ask = {
        let app = app.clone();
        tokio::spawn(async move {
            send(&app, post("/api/v1/vault/request", &[], json!({ "kind": "api-key", "label": "Stripe test key", "why": "run the payment tests", "wait_secs": 20 }))).await
        })
    };
    let mut req = None;
    for _ in 0..100 {
        req = requests::requests().list().into_iter().find(|r| r.label == "Stripe test key");
        if req.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let req = req.expect("the request waits for the person");
    assert_eq!(req.agent, "Something on this computer (not a verified agent)", "an unsigned request names no agent");
    assert_eq!(req.why, "run the payment tests");

    let fill = json!({ "op": "create", "request_id": req.id, "kind": "api-key", "label": "Stripe test key", "parts": { "value": "sk_test_abcdef123456" } });
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], fill.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an agent can't answer for the person");
    assert!(!ask.is_finished());
    let (s, body) = send(&app, post("/api/v1/host/vault", &[HOST], fill)).await;
    assert_eq!(s, StatusCode::OK, "{body}");

    let (s, answer) = ask.await.unwrap();
    assert_eq!(s, StatusCode::OK);
    assert_eq!(answer["status"], "created");
    assert_eq!(answer["id"], "api-key-stripe-test-key");
    assert!(!answer.to_string().contains("sk_test"), "{answer}");
}

#[tokio::test]
async fn a_declined_request_says_so() {
    let app = app().await;
    let ask = {
        let app = app.clone();
        tokio::spawn(async move { send(&app, post("/api/v1/vault/request", &[], json!({ "kind": "ssn", "label": "Declined one", "wait_secs": 20 }))).await })
    };
    let mut id = None;
    for _ in 0..100 {
        id = requests::requests().list().into_iter().find(|r| r.label == "Declined one").map(|r| r.id);
        if id.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (s, _) = send(&app, post("/api/v1/host/vault", &[HOST], json!({ "op": "decline_request", "request_id": id.unwrap() }))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, answer) = ask.await.unwrap();
    assert_eq!(answer["status"], "declined");
}

// §5.3: only the host says a Vault pane is showing; agents only ask.
#[tokio::test]
async fn only_the_host_says_a_vault_pane_is_showing() {
    let app = app().await;
    let on_screen = |app: axum::Router| async move { send(&app, get("/api/v1/vault/on_screen")).await.1["on_screen"].clone() };
    let note = |pane: &str, on: bool| json!({ "op": "on_screen", "pane": pane, "on": on });
    assert_eq!(on_screen(app.clone()).await, json!(false));
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], note("p1", true))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = send(&app, post("/api/v1/host/vault", &[HOST], note("p1", true))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(on_screen(app.clone()).await, json!(true));
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], note("p1", false))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an agent can't clear it");
    assert_eq!(on_screen(app.clone()).await, json!(true));
    send(&app, post("/api/v1/host/vault", &[HOST], note("p1", false))).await;
    assert_eq!(on_screen(app).await, json!(false));
}

// Two Vault panes: one hiding leaves the other's note in place.
#[tokio::test]
async fn one_pane_hiding_doesnt_clear_anothers_note() {
    let app = app().await;
    let on_screen = |app: axum::Router| async move { send(&app, get("/api/v1/vault/on_screen")).await.1["on_screen"].clone() };
    let note = |pane: &str, on: bool| json!({ "op": "on_screen", "pane": pane, "on": on });
    send(&app, post("/api/v1/host/vault", &[HOST], note("p1", true))).await;
    send(&app, post("/api/v1/host/vault", &[HOST], note("p2", true))).await;
    send(&app, post("/api/v1/host/vault", &[HOST], note("p1", false))).await;
    assert_eq!(on_screen(app.clone()).await, json!(true), "p2 still shows");
    send(&app, post("/api/v1/host/vault", &[HOST], note("p2", false))).await;
    assert_eq!(on_screen(app.clone()).await, json!(false));
    let (s, _) = send(&app, post("/api/v1/host/vault", &[HOST], json!({ "op": "on_screen", "on": true }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a note names its pane");
}

#[tokio::test]
async fn a_request_for_an_unknown_kind_or_without_a_label_is_refused() {
    let app = app().await;
    let (s, body) = send(&app, post("/api/v1/vault/request", &[], json!({ "kind": "rocket", "label": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("api-key"), "it lists the kinds");
    let (s, _) = send(&app, post("/api/v1/vault/request", &[], json!({ "kind": "ssn", "label": "  " }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// A malformed save is refused without echoing what it carried.
#[tokio::test]
async fn a_malformed_entry_is_refused_without_quoting_its_value() {
    let app = app().await;
    let bad = json!({ "op": "create", "kind": "ssn", "label": "Alex", "parts": { "value": 123456789 } });
    let (s, body) = send(&app, post("/api/v1/host/vault", &[HOST], bad)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(!body.to_string().contains("123456789"), "{body}");
}

// Only the host can fetch a value (after the OS confirmed the person); an agent holding the instance key can't.
#[tokio::test]
async fn only_the_host_can_fetch_a_value() {
    let app = app().await;
    let (_, body) = send(&app, post("/api/v1/host/vault", &[HOST], new_ssn())).await;
    let id = body["data"]["id"].as_str().unwrap().to_string();
    let ask = json!({ "op": "reveal", "id": id, "part": "value", "action": "copied" });
    let (s, body) = send(&app, post("/api/v1/host/vault", &[], ask.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(!body.to_string().contains("123-45"), "{body}");
    let (s, body) = send(&app, post("/api/v1/host/vault", &[HOST], ask)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["data"]["value"], "123-45-6789");
    // Nothing an agent can read carries it.
    let (_, listed) = send(&app, get("/api/v1/vault")).await;
    assert!(!listed.to_string().contains("123-45"));
    let (_, act) = send(&app, post("/api/v1/host/vault", &[HOST], json!({ "op": "activity" }))).await;
    assert!(!act.to_string().contains("copied"), "fetching for a copy isn't a copy: {act}");
    let report = json!({ "op": "record_copy", "id": id, "part": "value", "done": true });
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], report.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an agent can't write the audit trail");
    let (s, _) = send(&app, post("/api/v1/host/vault", &[HOST], report)).await;
    assert_eq!(s, StatusCode::OK);
    let (_, act) = send(&app, post("/api/v1/host/vault", &[HOST], json!({ "op": "activity" }))).await;
    assert!(act.to_string().contains("copied") && !act.to_string().contains("123-45"), "{act}");
}

// A use that needs a typed value takes it with the answer, requires it to allow, and an answer
// without one can still deny.
#[tokio::test]
async fn an_answer_to_a_request_for_a_value_carries_what_the_person_typed() {
    use crate::backend::vault::uses::{self, Decision, UseRequest};
    let app = app().await;
    let host = |body: Value| post("/api/v1/host/vault", &[HOST], body);
    let request = |id: &str| UseRequest {
        id: id.into(),
        agent: "Agent1".into(),
        entry_id: String::new(),
        entry_label: "Sam's SSN".into(),
        what: "Social Security number".into(),
        origin: "https://irs.gov".into(),
        field: "SSN".into(),
        offer_always: false,
        reason: "no entry yet".into(),
        needs_value: true,
        kind: "ssn".into(),
        command: String::new(),
        cwd: String::new(),
        replaces: false,
        requested_ms: agentmux_common::time::now_ms_u64(),
    };
    let rx = uses::uses().ask(request("typed-1"));
    let (s, body) = send(&app, host(json!({ "op": "answer_use", "id": "typed-1", "decision": "once" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "no value: {body}");
    assert!(body.to_string().contains("type the value"));
    let (s, body) = send(&app, host(json!({ "op": "answer_use", "id": "typed-1", "decision": "once", "value": "987-65-4321", "save": true }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(!body.to_string().contains("987-65"));
    let got = rx.await.unwrap();
    assert_eq!((got.decision, got.value.as_deref().map(String::as_str), got.save), (Decision::Once, Some("987-65-4321"), true));

    let rx = uses::uses().ask(request("typed-2"));
    let (s, _) = send(&app, host(json!({ "op": "answer_use", "id": "typed-2", "decision": "deny" }))).await;
    assert_eq!(s, StatusCode::OK, "denying needs no value");
    assert_eq!(rx.await.unwrap().decision, Decision::Deny);
}

// The person's answer to an agent's use of an entry comes only from the host, "always" only
// when it was offered, and a grant can be revoked.
#[tokio::test]
async fn answering_a_use_and_revoking_a_grant_go_through_the_host_only() {
    use crate::backend::vault::uses::{self, Decision, UseRequest};
    let app = app().await;
    let host = |body: Value| post("/api/v1/host/vault", &[HOST], body);
    let use_req = |id: &str, offer_always: bool| UseRequest {
        id: id.into(),
        agent: "Agent1".into(),
        entry_id: "ssn-alex".into(),
        entry_label: "Alex".into(),
        what: "Social Security number".into(),
        origin: "https://irs.gov".into(),
        field: "SSN".into(),
        offer_always,
        reason: "a new site".into(),
        needs_value: false,
        kind: "ssn".into(),
        command: String::new(),
        cwd: String::new(),
        replaces: false,
        requested_ms: agentmux_common::time::now_ms_u64(),
    };

    let rx = uses::uses().ask(use_req("answer-1", true));
    let answer = json!({ "op": "answer_use", "id": "answer-1", "decision": "always" });
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], answer.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an agent can't answer for the person");
    let (s, body) = send(&app, host(answer.clone())).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(rx.await.unwrap().decision, Decision::Always);
    let (s, body) = send(&app, host(answer)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "answered already: {body}");

    let rx = uses::uses().ask(use_req("answer-2", false));
    let (s, body) = send(&app, host(json!({ "op": "answer_use", "id": "answer-2", "decision": "always" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("isn't offered"), "{body}");
    let (s, _) = send(&app, host(json!({ "op": "answer_use", "id": "answer-2", "decision": "sometimes" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&app, host(json!({ "op": "answer_use", "id": "answer-2", "decision": "deny" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(rx.await.unwrap().decision, Decision::Deny);

    // Revoking a grant that isn't there is refused, and an agent can't revoke at all.
    let (_, made) = send(&app, host(new_ssn())).await;
    let id = made["data"]["id"].as_str().unwrap().to_string();
    let revoke = json!({ "op": "revoke_grant", "entry_id": id, "origin": "https://irs.gov" });
    let (s, body) = send(&app, host(revoke.clone())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "no such grant yet: {body}");
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], revoke)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

// Export and import go through the host only, and a value is never in what comes back.
#[tokio::test]
async fn export_and_import_work_through_the_host_without_a_value_in_any_answer() {
    let app = app().await;
    let host = |body: Value| post("/api/v1/host/vault", &[HOST], body);
    send(&app, host(new_ssn())).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.vault").to_string_lossy().into_owned();
    let export = json!({ "op": "export", "path": path, "passphrase": "correct horse battery staple" });

    let (s, _) = send(&app, post("/api/v1/host/vault", &[], export.clone())).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an agent can't export");
    let (s, body) = send(&app, host(json!({ "op": "export", "path": "relative.vault", "passphrase": "correct horse battery staple" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    let (s, body) = send(&app, host(export)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!((body["data"]["count"].clone(), body["data"]["file_name"].clone()), (json!(1), json!("out.vault")));

    let (_, body) = send(&app, host(json!({ "op": "import_inspect", "path": path }))).await;
    assert_eq!(body["data"]["needs_passphrase"], true);
    let (s, body) = send(&app, host(json!({ "op": "import_inspect", "path": path, "passphrase": "wrong wrong wrong" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("didn't open"));
    let (_, body) = send(&app, host(json!({ "op": "import_inspect", "path": path, "passphrase": "correct horse battery staple" }))).await;
    assert!(!body.to_string().contains("123-45"), "{body}");
    let token = body["data"]["preview"]["token"].as_str().unwrap().to_string();
    assert_eq!(body["data"]["preview"]["duplicates"], 1);
    let (s, body) = send(&app, host(json!({ "op": "import_commit", "token": token, "duplicates": "keep_both" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["result"]["added"], 1);
    assert!(!body.to_string().contains("123-45"));
    let (_, listed) = send(&app, get("/api/v1/vault")).await;
    assert_eq!(listed["entries"].as_array().unwrap().len(), 2);
    let (s, body) = send(&app, host(json!({ "op": "import_commit", "token": "nope", "duplicates": "skip" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(body.to_string().contains("expired"));
    let (_, p) = send(&app, host(json!({ "op": "suggest_passphrase" }))).await;
    assert_eq!(p["data"]["passphrase"].as_str().unwrap().split('-').count(), 8);
}

// Warden's list of standing permissions is metadata only: the host reads it, an agent can't, and it
// carries labels and grants, never a value.
#[tokio::test]
async fn the_standing_permissions_are_the_hosts_to_read_and_carry_no_value() {
    let app = app().await;
    let host = |body: Value| post("/api/v1/host/vault", &[HOST], body);
    send(&app, host(new_ssn())).await;
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], json!({ "op": "grants" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, body) = send(&app, host(json!({ "op": "grants" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(body["data"]["granted"].is_array() && !body.to_string().contains("123-45"), "{body}");
}

// A key file the host chose but that is empty is refused as a key file, not silently treated as none.
#[tokio::test]
async fn an_empty_key_file_is_a_refused_key_file_and_not_no_key_file() {
    let app = app().await;
    let host = |body: Value| post("/api/v1/host/vault", &[HOST], body);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Keyed.kdbx");
    std::fs::write(&path, include_bytes!("../../backend/vault/import/fixtures/pykeepass_keyfile_only.kdbx")).unwrap();
    let p = path.to_string_lossy().to_string();
    let (s, none) = send(&app, host(json!({ "op": "import_inspect", "path": p }))).await;
    assert_eq!((s, none["data"]["needs_passphrase"].as_bool()), (StatusCode::OK, Some(true)));
    let (s, empty) = send(&app, host(json!({ "op": "import_inspect", "path": p, "keyfile": "" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{empty}");
    assert!(empty.to_string().contains("key file"), "{empty}");
}

// The export is a `.vault` file or, asked for, a KeePass database: the host's route only, and nothing
// else is accepted as a format.
#[tokio::test]
async fn an_export_can_be_a_keepass_database_and_only_a_known_format() {
    let app = app().await;
    let host = |body: Value| post("/api/v1/host/vault", &[HOST], body);
    send(&app, host(new_ssn())).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.kdbx").to_string_lossy().into_owned();
    let (s, _) = send(&app, post("/api/v1/host/vault", &[], json!({ "op": "export", "path": path, "format": "kdbx", "passphrase": "correct horse battery staple" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an agent can't export in any format");
    let (s, body) = send(&app, host(json!({ "op": "export", "path": path, "format": "zip", "passphrase": "correct horse battery staple" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    let (s, body) = send(&app, host(json!({ "op": "export", "path": path, "format": "kdbx", "passphrase": "correct horse battery staple" }))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], &[0x03, 0xD9, 0xA2, 0x9A]);
    assert!(!body.to_string().contains("123-45"), "{body}");
}

/// A state and a signed identity for a registered agent, as `Vault` `generate` sends.
fn signed_agent(state: &AppState) -> agentmux_common::api_types::UiAutomationAuth {
    let agent = format!("maker-{}", uuid::Uuid::new_v4());
    let block = format!("block-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&agent, &block, None).unwrap();
    let key = state.mstore.agent_jekt_key_ensure(&agent).unwrap();
    let ts = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", &agent, "__srv__", ts, "");
    agentmux_common::api_types::UiAutomationAuth { agent_id: agent, ts_secs: ts, sig }
}

// §4.1 of the generate spec: the answer names the entry and never holds its value.
#[tokio::test]
async fn generate_answers_with_the_entry_and_never_the_value() {
    crate::backend::vault::seal::use_test_key();
    let state = crate::server::tests::test_state();
    let auth = signed_agent(&state);
    let app = crate::server::build_router(state.clone());
    let (s, body) = send(
        &app,
        post("/api/v1/vault/generate", &[], json!({ "kind": "api-key", "label": "Webhook signing secret", "why": "sign test webhooks", "auth": auth })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let id = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["status"], "created");
    let value = crate::backend::vault::reveal(&state.identity_store, &id, "value", "revealed").unwrap();
    assert!(!body.to_string().contains(value.as_str()), "the answer holds the value");
    let rows = crate::backend::vault::activity(&state.identity_store, 10).unwrap();
    assert!(rows.iter().any(|r| r.action == "generated" && r.actor == auth.agent_id));
}

#[tokio::test]
async fn generate_refuses_a_caller_without_a_proven_identity() {
    let app = app().await;
    let (s, body) = send(&app, post("/api/v1/vault/generate", &[], json!({ "kind": "api-key", "label": "x" }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    let forged = json!({ "agent_id": "AgentY", "ts_secs": agentmux_common::time::now_secs(), "sig": "00" });
    let (s, _) = send(&app, post("/api/v1/vault/generate", &[], json!({ "kind": "api-key", "label": "x", "auth": forged }))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, list) = send(&app, get("/api/v1/vault")).await;
    assert_eq!(list["entries"], json!([]));
}

// QTest, 2026-10-27: a request asked again after the session ended opened a second form.
// The same agent asking for the same entry gets the one request, then its answer.
#[tokio::test]
async fn asking_again_for_the_same_entry_opens_no_second_form() {
    crate::backend::vault::seal::use_test_key();
    let state = crate::server::tests::test_state();
    *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
    let auth = signed_agent(&state);
    let app = crate::server::build_router(state);
    let label = format!("Store client secret {}", uuid::Uuid::new_v4().simple());
    let ask = json!({ "kind": "api-key", "label": label, "why": "sign the Store upload", "wait_secs": 0, "auth": auth });

    let (s, first) = send(&app, post("/api/v1/vault/request", &[], ask.clone())).await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!(first["status"], "pending");
    let (_, second) = send(&app, post("/api/v1/vault/request", &[], ask.clone())).await;
    assert_eq!(second["status"], "pending");
    assert_eq!(second["request_id"], first["request_id"], "the retry joins the open request");
    assert_eq!(requests::requests().list().iter().filter(|r| r.label == label).count(), 1, "one form");

    let fill = json!({ "op": "create", "request_id": first["request_id"], "kind": "api-key", "label": label, "parts": { "value": "client-secret-value-123" } });
    let (s, body) = send(&app, post("/api/v1/host/vault", &[HOST], fill)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (_, third) = send(&app, post("/api/v1/vault/request", &[], ask)).await;
    assert_eq!(third["status"], "created", "{third}");
    assert_eq!(third["id"], body["data"]["id"]);
    assert!(!third.to_string().contains("client-secret-value"));
}
