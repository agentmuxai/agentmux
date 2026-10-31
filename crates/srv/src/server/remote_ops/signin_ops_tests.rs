// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use super::super::{lookup, Verification};
use super::*;
use crate::server::tests::test_state;

fn caller(slug: &str) -> RemoteCaller {
    RemoteCaller {
        agent_uid: "uid-claimed-by-the-requester".into(),
        slug: slug.into(),
        machine: "narko".into(),
        instance_id: "inst-1".into(),
        install_trusted: true,
        verification: Verification::Lan,
        reason: "the account is out".into(),
    }
}

#[test]
fn every_signin_op_needs_an_approved_install() {
    for name in ["identity.signin.start", "identity.signin.finish"] {
        let op = lookup(name).unwrap_or_else(|| panic!("{name} isn't registered"));
        assert!(op.changes_state && op.needs_approved_install, "{name}");
    }
}

#[tokio::test]
async fn only_claude_can_be_signed_in_so_far() {
    let state = test_state();
    let refused = start(&state, &caller("agenty"), json!({ "provider": "codex" })).await.unwrap_err();
    assert_eq!(refused.code, "bad_params");
    assert!(refused.message.contains("only claude"), "{}", refused.message);
}

/// A sign-in is the caller's who started it; the answer carries the link, then the account's id
/// and email, and never a directory or a token.
#[tokio::test]
async fn a_session_belongs_to_the_caller_and_never_shows_a_path_or_a_token() {
    let state = test_state();
    let mgr = &state.auth_session_manager;
    let session = mgr.start_session("claude".into(), None).session_id;
    remember(&state, &session, &caller("agenty").actor());

    // Someone else, on the same machine or not, can't read it or finish it.
    for other in [caller("naki"), RemoteCaller { machine: "area54".into(), ..caller("agenty") }] {
        let refused = finish(&state, &other, json!({ "session": session, "code": "x" })).await.unwrap_err();
        assert!(refused.message.contains("no sign-in of yours"), "{}", refused.message);
    }

    assert!(mgr.finish_success(&session, String::new(), Some("acct-1".into())));
    let done = finish(&state, &caller("agenty"), json!({ "session": session })).await.unwrap();
    assert_eq!(done["state"], "signed_in");
    assert_eq!(done["account_id"], "acct-1");
    let text = done.to_string();
    for secret in ["token", "dir", "path", "CLAUDE_CONFIG_DIR"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
}

#[tokio::test]
async fn a_code_for_a_login_that_is_not_listening_is_refused() {
    let state = test_state();
    let session = state.auth_session_manager.start_session("claude".into(), None).session_id;
    remember(&state, &session, &caller("agenty").actor());
    let refused = finish(&state, &caller("agenty"), json!({ "session": session, "code": "abc" })).await.unwrap_err();
    assert!(refused.message.contains("isn't waiting for a code"), "{}", refused.message);
}

/// A re-sign-in names an account of that provider on this machine; any other id is refused before a
/// directory is made for it or another account's tokens are replaced.
#[tokio::test]
async fn signing_an_account_in_again_needs_an_existing_account_of_that_provider() {
    use crate::backend::storage::identities::{IdentityAccount, SecretRef};
    let state = test_state();
    let other = IdentityAccount {
        id: "cx".to_string(),
        name: "cx".to_string(),
        provider: "codex".to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: "/tmp/cx".to_string() },
        context: json!({}),
        status: "valid".to_string(),
        created_at: 0,
        updated_at: 0,
    };
    state.id_store.identity_upsert(&other).unwrap();
    for id in ["no-such-account", "cx"] {
        let refused = start(&state, &caller("agenty"), json!({ "account": id })).await.unwrap_err();
        assert_eq!(refused.code, "bad_params", "{id}");
        assert!(refused.message.contains("isn't a claude account"), "{}", refused.message);
    }
}

/// Owners of sessions srv no longer holds are dropped; a live one is never evicted by age alone.
#[tokio::test]
async fn owners_of_gone_sessions_are_dropped_before_a_live_one_is_evicted() {
    let state = test_state();
    let live = state.auth_session_manager.start_session("claude".into(), None).session_id;
    remember(&state, &live, &caller("agenty").actor());
    for i in 0..MAX_OWNED + 5 {
        remember(&state, &format!("gone-{i}"), "other@x");
    }
    assert!(owned_by(&state, &live, &caller("agenty").actor()));
}
