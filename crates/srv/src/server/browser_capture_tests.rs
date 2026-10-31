// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::store::Store;
use crate::backend::vault::{activity, create, reveal, seal, versions, NewEntry};

const SECRET: &str = "Xy8Q~abcDEF1234ghijKLmnoPQRstu.vwxYZ0123";

fn store() -> Store {
    seal::use_test_key();
    let s = Store::open_in_memory().unwrap();
    s.apply_identity_schema_for_tests().unwrap();
    s
}

fn auth() -> UiAutomationAuth {
    UiAutomationAuth { agent_id: "Agent2-6".into(), ts_secs: 0, sig: String::new() }
}

fn request(into: Option<CaptureAs>, entry: Option<&str>) -> CaptureRequest {
    CaptureRequest { auth: auth(), pane: None, ref_: Some("e3".into()), into, entry: entry.map(String::from), download: false, timeout_ms: None }
}

fn new_as(kind: Option<&str>, label: &str) -> CaptureAs {
    CaptureAs { kind: kind.map(String::from), label: label.into(), ..Default::default() }
}

fn planned(s: &Store, req: &CaptureRequest) -> capture::Capture {
    capture::plan(s, &[rule_of(req).unwrap()], "Agent2-6", "").unwrap()
}

#[test]
fn the_request_names_a_new_entry_or_an_existing_one() {
    let new = rule_of(&request(Some(new_as(None, "GitHub OAuth secret")), None)).unwrap();
    assert_eq!((new.into.kind.as_deref(), new.into.label.as_deref()), (Some("api-key"), Some("GitHub OAuth secret")), "api-key by default");
    assert_eq!(rule_of(&request(None, Some("api-key-x"))).unwrap().into.id.as_deref(), Some("api-key-x"));
    assert!(rule_of(&request(Some(new_as(None, "x")), Some("api-key-x"))).unwrap_err().contains("not both"));
    assert!(rule_of(&request(None, None)).unwrap_err().contains("say where it goes"));
}

#[test]
fn a_downloaded_file_is_a_note_unless_the_agent_says_otherwise() {
    let file = CaptureRequest { download: true, ..request(Some(new_as(None, "GitHub App private key")), None) };
    assert_eq!(rule_of(&file).unwrap().into.kind.as_deref(), Some("note"));
    let as_key = CaptureRequest { download: true, ..request(Some(new_as(Some("api-key"), "Key")), None) };
    assert_eq!(rule_of(&as_key).unwrap().into.kind.as_deref(), Some("api-key"));
}

#[test]
fn a_downloaded_files_text_is_stored_as_the_notes_text() {
    let s = store();
    let req = CaptureRequest { download: true, ..request(Some(new_as(None, "GitHub App private key")), None) };
    let text = "line one\nline two of the file";
    let (id, _) = store_captured(&s, planned(&s, &req), Zeroizing::new(text.into()), "https://github.com", req.into.as_ref()).unwrap();
    assert_eq!(reveal(&s, &id, "text", "revealed").unwrap().as_str(), text);
    assert!(DOWNLOADED_NOTE.contains("never written to disk"));
}

#[test]
fn the_value_leaves_the_hosts_answer_for_wiped_memory() {
    let (value, origin) = take_read(json!({ "value": SECRET, "origin": "https://github.com", "page": "https://github.com/settings" })).unwrap();
    assert_eq!((value.as_str(), origin.as_str()), (SECRET, "https://github.com"));
    assert!(take_read(json!({ "origin": "https://github.com" })).is_err());
}

#[test]
fn the_answer_promises_screenshots_and_eval_only_when_the_element_was_marked() {
    assert!(captured_note(true).contains("eval is refused"));
    let unmarked = captured_note(false);
    assert!(!unmarked.contains("eval is refused") && !unmarked.contains("screenshots"), "{unmarked}");
}

#[test]
fn a_new_entry_keeps_the_pages_origin_and_says_where_it_came_from() {
    let s = store();
    let req = request(Some(CaptureAs { expires: Some("2027-10-30".into()), ..new_as(None, "GitHub OAuth secret") }), None);
    let (id, label) = store_captured(&s, planned(&s, &req), Zeroizing::new(SECRET.into()), "https://github.com", req.into.as_ref()).unwrap();
    assert_eq!((id.as_str(), label.as_str()), ("api-key-github-oauth-secret", "GitHub OAuth secret"));
    assert_eq!(reveal(&s, &id, "value", "revealed").unwrap().as_str(), SECRET);
    let row = s.vault_get(&id).unwrap().unwrap();
    let fields: BTreeMap<String, String> = serde_json::from_str(&row.fields).unwrap();
    assert_eq!(fields.get("service").map(String::as_str), Some("https://github.com"));
    assert_eq!(fields.get("expires").map(String::as_str), Some("2027-10-30"));
    let made = activity(&s, 10).unwrap().into_iter().find(|a| a.entry_id == id && a.action == "captured").unwrap();
    assert_eq!((made.actor.as_str(), made.target.as_str()), ("Agent2-6", "from https://github.com"));
}

#[test]
fn a_service_the_agent_names_is_kept_and_a_login_keeps_the_origin_as_its_site() {
    let s = store();
    let mut named = new_as(None, "Azure app secret");
    named.fields.insert("service".into(), "Entra app registration".into());
    let req = request(Some(named), None);
    let (id, _) = store_captured(&s, planned(&s, &req), Zeroizing::new(SECRET.into()), "https://portal.azure.com", req.into.as_ref()).unwrap();
    let fields: BTreeMap<String, String> = serde_json::from_str(&s.vault_get(&id).unwrap().unwrap().fields).unwrap();
    assert_eq!(fields.get("service").map(String::as_str), Some("Entra app registration"));

    let req = request(Some(new_as(Some("login"), "Console")), None);
    let (id, _) = store_captured(&s, planned(&s, &req), Zeroizing::new(SECRET.into()), "https://console.example", req.into.as_ref()).unwrap();
    let fields: BTreeMap<String, String> = serde_json::from_str(&s.vault_get(&id).unwrap().unwrap().fields).unwrap();
    assert_eq!(fields.get("site").map(String::as_str), Some("https://console.example"));
}

#[test]
fn a_capture_into_an_existing_entry_is_its_new_value_and_keeps_the_old_one() {
    let s = store();
    let id = create(&s, NewEntry { kind: "api-key".into(), label: "GitHub OAuth secret".into(), fields: Default::default(), tags: vec![], parts: seal::Parts([("value".to_string(), "old_secret_value_0001".to_string())].into()) }).unwrap();
    let req = request(None, Some(&id));
    let (got, _) = store_captured(&s, planned(&s, &req), Zeroizing::new(SECRET.into()), "https://github.com", None).unwrap();
    assert_eq!(got, id);
    assert_eq!(reveal(&s, &id, "value", "revealed").unwrap().as_str(), SECRET);
    assert_eq!(versions::list(&s, &id).unwrap().len(), 1);
}
