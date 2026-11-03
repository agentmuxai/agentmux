// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::google_grants::GrantLevel;

fn scope(name: &str) -> String {
    format!("https://www.googleapis.com/auth/{name}")
}

#[test]
fn a_connect_asks_for_the_services_named_and_everything_the_account_already_holds() {
    // D3: the union, every time, so a token that doesn't carry earlier grants over still holds them all.
    let held = vec![scope("calendar.events.readonly"), scope("drive.file")];
    let asked = union_scopes(&[Service::Gmail], GrantLevel::Read, &held);
    assert_eq!(asked, [scope("calendar.events.readonly"), scope("drive.file"), scope("gmail.readonly")]);
    // A scope the account holds and the service also needs is asked once.
    let again = union_scopes(&[Service::Drive, Service::Gmail], GrantLevel::Read, &held);
    assert_eq!(again.iter().filter(|s| **s == scope("drive.file")).count(), 1);
    assert!(again.contains(&scope("gmail.readonly")));
}

#[test]
fn a_write_level_adds_the_write_scopes_of_each_service() {
    let asked = union_scopes(&[Service::Gmail], GrantLevel::Write, &[]);
    assert_eq!(asked, [scope("gmail.readonly"), scope("gmail.compose"), scope("gmail.modify")]);
}

#[test]
fn the_pane_may_stay_on_googles_sites_and_the_loopback_callback_and_nowhere_else() {
    let url = "https://accounts.google.com/o/oauth2/v2/auth?response_type=code&client_id=cid\
               &redirect_uri=http%3A%2F%2F127.0.0.1%3A51234%2Fcallback&scope=x&state=s";
    let origins = pane_origins(url);
    assert_eq!(origins, ["*.google.com", "*.gstatic.com", "*.googleusercontent.com", "http://127.0.0.1:51234"]);
    // A URL without a redirect adds no origin.
    assert_eq!(pane_origins("https://accounts.google.com/o/oauth2/v2/auth").len(), 3);
}

#[test]
fn services_are_named_one_of_the_three_and_at_least_one() {
    assert!(parse_services(&[]).unwrap_err().contains("services is required"));
    assert!(parse_services(&["mail".to_string()]).unwrap_err().contains("calendar, drive or gmail"));
    let parsed = parse_services(&["Gmail".to_string(), "calendar".to_string(), "gmail".to_string()]).unwrap();
    assert_eq!(parsed, [Service::Gmail, Service::Calendar]);
}

#[test]
fn only_the_agent_that_started_a_connect_may_ask_after_it() {
    owners().insert("sess-owned".to_string(), "agent-a".to_string());
    assert!(owns("agent-a", "sess-owned").is_ok());
    assert!(owns("agent-b", "sess-owned").unwrap_err().contains("another agent"));
    assert!(owns("agent-a", "sess-none").unwrap_err().contains("no connect with that session"));
    owners().remove("sess-owned");
}

#[tokio::test]
async fn a_connect_that_waits_blocks_a_second_one_from_the_same_agent() {
    let store = std::sync::Arc::new(crate::backend::storage::store::Store::open_in_memory().unwrap());
    let opts = StartOptions {
        target: FlowTarget::New { name: String::new() },
        byo: Some(oauth_client::ByoCredentials {
            client_id: "cid".into(),
            client_secret: Some("csec".into()),
            mode: crate::identity::oauth_tokens::ClientMode::Byo,
        }),
        saved: None,
        extra_scopes: Vec::new(),
        save_client: false,
        cloud_signed_in: false,
        replace_user: false,
        open_system_browser: false,
        login_hint: None,
    };
    let (session, _) = oauth_client::start("google", opts, store.clone(), store).unwrap();
    owners().insert(session.clone(), "agent-wait".to_string());
    assert_eq!(waiting_session_of("agent-wait"), Some(session.clone()));
    assert_eq!(waiting_session_of("agent-other"), None);
    oauth_client::manager().cancel(&session);
    assert_eq!(waiting_session_of("agent-wait"), None, "a cancelled connect no longer waits");
    owners().remove(&session);
}
