// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The auth-login spawn engine against a stub provider CLI (`sh -c <script>`): how a session ends for each way the
//! CLI and its auth check can go, on the piped-stdio path and on the PTY path. Unix only: the stub is `sh`.
#![cfg(unix)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::*;
use crate::identity::auth_session::{AuthSessionManager, AuthSessionStatus};

struct Ran {
    status: AuthSessionStatus,
    mstore: Arc<Store>,
}

fn spawn(
    mgr: &Arc<AuthSessionManager>,
    mstore: &Arc<Store>,
    tty: bool,
    cli: &str,
    login: &str,
    check: &str,
    dir: &tempfile::TempDir,
) -> String {
    let session_id = mgr.start_session("claude".to_string(), None).session_id;
    spawn_auth_cli(
        mgr.clone(),
        mstore.clone(),
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(Broker::new()),
        session_id.clone(),
        "claude".to_string(),
        None,
        Some(dir.path().to_string_lossy().to_string()),
        true,
        "acc-1".to_string(),
        cli.to_string(),
        vec!["-c".to_string(), login.to_string()],
        vec!["-c".to_string(), check.to_string()],
        HashMap::new(),
        tty,
    );
    session_id
}

async fn until(mgr: &Arc<AuthSessionManager>, session_id: &str, what: &str, done: impl Fn(&AuthSessionStatus) -> bool) -> AuthSessionStatus {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(p) = mgr.poll_session(session_id) {
            if done(&p.status) {
                return p.status;
            }
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Run the stub to a terminal state.
async fn run(tty: bool, cli: &str, login: &str, check: &str) -> Ran {
    let mgr = Arc::new(AuthSessionManager::new());
    let mstore = Arc::new(Store::open_in_memory().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let session_id = spawn(&mgr, &mstore, tty, cli, login, check, &dir);
    let status = until(&mgr, &session_id, "a terminal state", |s| s.is_terminal()).await;
    Ran { status, mstore }
}

const SH: &str = "/bin/sh";

#[tokio::test(flavor = "multi_thread")]
async fn a_login_line_and_a_passing_check_save_the_account_and_succeed() {
    for tty in [false, true] {
        let ran = run(tty, SH, "echo 'Logged in as bob@example.com'", "exit 0").await;
        match &ran.status {
            AuthSessionStatus::Success { bundle_id, email, account_id } => {
                assert_eq!(bundle_id, "", "tty={tty}");
                assert_eq!(account_id.as_deref(), Some("acc-1"), "tty={tty}");
                assert_eq!(email.as_deref(), Some("bob@example.com"), "tty={tty}");
            }
            other => panic!("tty={tty}: {other:?}"),
        }
        assert!(ran.mstore.identity_get("acc-1").unwrap().is_some(), "tty={tty}: saved");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cli_that_exits_clean_without_a_login_line_succeeds_when_the_check_passes() {
    for tty in [false, true] {
        let ran = run(tty, SH, "exit 0", "exit 0").await;
        assert!(
            matches!(&ran.status, AuthSessionStatus::Success { account_id, .. } if account_id.as_deref() == Some("acc-1")),
            "tty={tty}: {:?}",
            ran.status
        );
        assert!(ran.mstore.identity_get("acc-1").unwrap().is_some(), "tty={tty}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clean_exit_whose_check_fails_is_a_failure_and_saves_nothing() {
    for (tty, want) in [
        (false, "CLI exited 0 but authentication check failed"),
        (true, "CLI exited cleanly but auth-check still failed"),
    ] {
        let ran = run(tty, SH, "exit 0", "exit 1").await;
        assert!(matches!(&ran.status, AuthSessionStatus::Failed { error } if error == want), "tty={tty}: {:?}", ran.status);
        assert!(ran.mstore.identity_get("acc-1").unwrap().is_none(), "tty={tty}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_line_whose_check_fails_is_still_a_failure() {
    for tty in [false, true] {
        let ran = run(tty, SH, "echo 'Logged in as bob@example.com'", "exit 1").await;
        assert!(matches!(&ran.status, AuthSessionStatus::Failed { .. }), "tty={tty}: {:?}", ran.status);
        assert!(ran.mstore.identity_get("acc-1").unwrap().is_none(), "tty={tty}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cli_that_exits_with_an_error_fails_with_its_code() {
    for (tty, want) in [(false, "CLI exited with status exit status: 3"), (true, "CLI exited with code 3")] {
        let ran = run(tty, SH, "exit 3", "exit 0").await;
        assert!(matches!(&ran.status, AuthSessionStatus::Failed { error } if error == want), "tty={tty}: {:?}", ran.status);
        assert!(ran.mstore.identity_get("acc-1").unwrap().is_none(), "tty={tty}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cli_that_cannot_be_started_fails_the_session_naming_it() {
    for (tty, prefix) in [(false, "spawn `/nonexistent/cli` failed"), (true, "PTY spawn `/nonexistent/cli` failed")] {
        let ran = run(tty, "/nonexistent/cli", "exit 0", "exit 0").await;
        assert!(matches!(&ran.status, AuthSessionStatus::Failed { error } if error.starts_with(prefix)), "tty={tty}: {:?}", ran.status);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_url_is_surfaced_while_the_cli_runs_and_cancelling_stops_it() {
    for tty in [false, true] {
        let mgr = Arc::new(AuthSessionManager::new());
        let mstore = Arc::new(Store::open_in_memory().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let session_id = spawn(&mgr, &mstore, tty, SH, "echo 'Open https://example.com/oauth/authorize?x=1 to sign in'; sleep 30", "exit 1", &dir);

        let status = until(&mgr, &session_id, "the url", |s| matches!(s, AuthSessionStatus::UrlAvailable { .. })).await;
        assert!(
            matches!(&status, AuthSessionStatus::UrlAvailable { auth_url } if auth_url.starts_with("https://example.com/oauth/authorize")),
            "tty={tty}: {status:?}"
        );

        assert!(mgr.cancel_session(&session_id), "tty={tty}");
        assert!(mstore.identity_get("acc-1").unwrap().is_none(), "tty={tty}: nothing saved");
    }
}
