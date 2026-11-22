// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::time::now_secs;
use base64::Engine;

use crate::backend::transcript::withhold;
use crate::identity::resolver::probe_oauth_status;

fn hidden(value: &str) -> bool {
    withhold::text(&format!("v {value}")) == format!("v {}", withhold::MARK)
}

fn shown(value: &str) -> bool {
    withhold::text(&format!("v {value}")) == format!("v {value}")
}

/// A synthetic JWT that expires at `exp_secs`, whose signature part is `canary`.
fn jwt(exp_secs: i64, canary: &str) -> String {
    let b64 = |s: String| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
    format!("{}.{}.{canary}", b64("{\"alg\":\"none\"}".into()), b64(format!("{{\"exp\":{exp_secs}}}")))
}

fn write(path: &std::path::Path, body: serde_json::Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body.to_string()).unwrap();
}

fn look(provider: &str, dir: &std::path::Path) {
    probe_oauth_status(provider, dir.to_str().unwrap(), agentmux_common::time::now_ms());
}

fn codex_file(access: &str, id: &str, refresh: &str, api_key: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "OPENAI_API_KEY": api_key,
        "tokens": { "access_token": access, "id_token": id, "refresh_token": refresh },
    })
}

#[test]
fn a_codex_sign_in_is_withheld_after_the_look_and_a_renewed_one_leaves_the_old_access_token_withheld() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("auth.json");
    let (a1, i1) = (jwt(now_secs() + 3_600, "canary-codex-access-0001"), jwt(now_secs() + 3_600, "canary-codex-id-0001"));
    write(&path, codex_file(&a1, &i1, "canary-codex-refresh-0001", Some("canary-codex-apikey-0001")));
    assert!(shown(&a1) && shown("canary-codex-refresh-0001"), "not read yet");

    look("codex", tmp.path());
    for value in [&a1, &i1, &"canary-codex-refresh-0001".to_string(), &"canary-codex-apikey-0001".to_string()] {
        assert!(hidden(value), "{value}");
    }

    // The CLI renews the tokens: the new ones are withheld; the old access token still works until its exp.
    let (a2, i2) = (jwt(now_secs() + 7_200, "canary-codex-access-0002"), jwt(now_secs() + 7_200, "canary-codex-id-0002"));
    write(&path, codex_file(&a2, &i2, "canary-codex-refresh-0002", Some("canary-codex-apikey-0002")));
    look("codex", tmp.path());
    for value in [&a2, &i2, &a1, &i1, &"canary-codex-refresh-0002".to_string(), &"canary-codex-apikey-0002".to_string()] {
        assert!(hidden(value), "{value}");
    }
    assert!(hidden("canary-codex-refresh-0001"), "the replaced refresh token is kept");
    assert!(hidden("canary-codex-apikey-0001"), "the replaced API key is kept");
}

#[test]
fn a_codex_token_that_is_not_a_jwt_stays_withheld_after_the_file_holds_another() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("auth.json");
    write(&path, codex_file("canary-codex-opaque-0001", "", "canary-codex-opaque-r-0001", None));
    look("codex", tmp.path());
    assert!(hidden("canary-codex-opaque-0001") && hidden("canary-codex-opaque-r-0001"));
    write(&path, codex_file("canary-codex-opaque-0002", "", "canary-codex-opaque-r-0002", None));
    look("codex", tmp.path());
    assert!(hidden("canary-codex-opaque-0002"));
    assert!(hidden("canary-codex-opaque-0001") && hidden("canary-codex-opaque-r-0001"));
}

#[test]
fn an_api_key_the_file_no_longer_holds_stays_withheld_on_every_read_path() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("auth.json");
    write(&path, codex_file("", "", "", Some("canary-keep-apikey-A-0001")));
    look("codex", tmp.path());
    write(&path, codex_file("", "", "", Some("canary-keep-apikey-B-0001")));
    look("codex", tmp.path());
    assert!(hidden("canary-keep-apikey-A-0001") && hidden("canary-keep-apikey-B-0001"));
    // The same on the other read paths: the status probe for Claude's file shape, and grok's and muse's files.
    let grok = tempfile::tempdir().unwrap();
    for n in ["A", "B"] {
        write(&grok.path().join("auth.json"), serde_json::json!({ "s": { "key": format!("canary-keep-grok-{n}-0001"), "refresh_token": format!("canary-keep-grokr-{n}-0001") } }));
        look("grok", grok.path());
    }
    assert!(hidden("canary-keep-grokr-A-0001") && hidden("canary-keep-grokr-B-0001"));
    let muse = tempfile::tempdir().unwrap();
    for n in ["A", "B"] {
        write(&muse.path().join("muse").join("auth.json"), serde_json::json!({ "api_key": format!("canary-keep-muse-{n}-0001") }));
        look("muse", muse.path());
    }
    assert!(hidden("canary-keep-muse-A-0001") && hidden("canary-keep-muse-B-0001"));
}

#[test]
fn a_provider_keeps_at_most_eight_replaced_values_and_drops_the_oldest() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("auth.json");
    for n in 0..10 {
        write(&path, codex_file("", "", &format!("canary-cap-refresh-{n:02}-value"), None));
        look("codex", tmp.path());
    }
    assert!(shown("canary-cap-refresh-00-value") && shown("canary-cap-refresh-01-value"));
    assert!(hidden("canary-cap-refresh-02-value") && hidden("canary-cap-refresh-09-value"));
}

#[test]
fn a_look_at_a_signed_out_codex_file_leaves_what_was_held() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("auth.json");
    write(&path, codex_file("", "", "canary-codex-signout-0001", None));
    look("codex", tmp.path());
    write(&path, serde_json::json!({ "tokens": {} }));
    look("codex", tmp.path());
    assert!(hidden("canary-codex-signout-0001"));
}

#[test]
fn a_grok_key_and_refresh_token_are_withheld_per_scope_and_a_renewed_key_leaves_the_old_one_withheld() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("auth.json");
    let at = |secs: i64| chrono::DateTime::from_timestamp(secs, 0).unwrap().to_rfc3339();
    write(
        &path,
        serde_json::json!({
            "scope-a": { "key": "canary-grok-key-a-0001", "expires_at": at(now_secs() + 3_600), "refresh_token": "canary-grok-refresh-a-0001" },
            "scope-b": { "key": "canary-grok-key-b-0001", "create_time": at(now_secs()) },
        }),
    );
    look("grok", tmp.path());
    for value in ["canary-grok-key-a-0001", "canary-grok-refresh-a-0001", "canary-grok-key-b-0001"] {
        assert!(hidden(value), "{value}");
    }

    write(
        &path,
        serde_json::json!({
            "scope-a": { "key": "canary-grok-key-a-0002", "expires_at": at(now_secs() + 7_200), "refresh_token": "canary-grok-refresh-a-0002" },
        }),
    );
    look("grok", tmp.path());
    assert!(hidden("canary-grok-key-a-0002") && hidden("canary-grok-refresh-a-0002"));
    assert!(hidden("canary-grok-key-a-0001"), "the replaced key works until its expiry");
    assert!(hidden("canary-grok-refresh-a-0001"), "the replaced refresh token is kept");
    assert!(hidden("canary-grok-key-b-0001"), "a scope that left the file is still held until its expiry");
}

#[test]
fn a_muse_credential_is_withheld_and_the_rest_of_its_file_is_not() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("muse").join("auth.json");
    write(
        &path,
        serde_json::json!({
            "display_name": "canary-muse-name-0001",
            "session": { "accessToken": "canary-muse-access-0001", "refresh": ["canary-muse-list-0001"] },
            "api_key": "canary-muse-apikey-0001",
        }),
    );
    look("muse", tmp.path());
    assert!(hidden("canary-muse-access-0001") && hidden("canary-muse-apikey-0001"));
    assert!(shown("canary-muse-name-0001"), "a field that is not a credential is not withheld");
    assert!(shown("canary-muse-list-0001"));

    write(&path, serde_json::json!({ "api_key": "canary-muse-apikey-0002" }));
    look("muse", tmp.path());
    assert!(hidden("canary-muse-apikey-0002"));
    assert!(hidden("canary-muse-apikey-0001"), "the replaced value is kept");
}
