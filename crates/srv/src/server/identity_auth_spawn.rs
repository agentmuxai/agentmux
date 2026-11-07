// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Subprocess spawn/drain engine for the pre-launch OAuth flow.
//!
//! Split out of `identity_handlers.rs` (module-organization pass, see
//! `docs/specs/PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md`) —
//! owns `spawn_auth_cli` (plain piped-stdio spawn), `spawn_auth_cli_pty`
//! (PTY-backed spawn for providers whose auth subcommand requires an
//! interactive TTY), and `confirm_authenticated` (the shared
//! authCheckCommand probe both spawn paths use). Both spawn paths call
//! into `identity_auth_persist::persist_oauth_success` on the OAuth-
//! success path.

use std::sync::Arc;

use crate::backend::mps::Broker;
use crate::backend::storage::store::Store;

#[path = "identity_auth_spawn_steps.rs"]
mod steps;

/// Spawn the provider's auth-login CLI and drive the session through
/// to a terminal state. Background-only — returns immediately. The
/// drain task feeds stdout+stderr lines into
/// `AuthSessionManager::record_line`; on a login-success pattern OR
/// child exit, runs the provider's authCheckCommand to confirm and
/// transitions to Success or Failed.
///
/// On the success path (CLI exited cleanly + authCheckCommand passed),
/// persists a standalone `IdentityAccount` (`persist_oauth_direct_account`)
/// with `SecretRef::OAuthConfigDir` pointing at `bundle_dir` (the
/// account's own isolation dir — see `compute_and_ensure_account_dir`),
/// status `valid`. `account_id` is the id minted/reused for that account.
/// After this point, future launches of any agent linked to the account
/// resolve through `inject_identity_env`'s oauth-class dispatch and reuse
/// the same CLI-managed tokens.
///
/// `into_bundle_id`/`direct_account` are vestigial parameters kept only
/// so the wire request shape and call sites don't need touching — bundle
/// mode (binding into `db_identity_bundles`) was retired in Phase 4c of
/// SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md; `persist_oauth_success`
/// always takes the direct-account path now.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_auth_cli(
    mgr: Arc<crate::identity::auth_session::AuthSessionManager>,
    mstore: Arc<Store>,
    identity_store: Arc<Store>,
    broker: Arc<Broker>,
    session_id: String,
    provider_id: String,
    into_bundle_id: Option<String>,
    bundle_dir: Option<String>,
    direct_account: bool,
    account_id: String,
    cli_path: String,
    auth_login_args: Vec<String>,
    auth_check_args: Vec<String>,
    auth_env: std::collections::HashMap<String, String>,
    requires_tty: bool,
) {
    use tokio::sync::mpsc;

    // Channel for SubmitAuthCallback → CLI stdin forwarding.
    // Buffer of 4 is enough — only one URL per session in normal use.
    let (stdin_tx, stdin_rx) = mpsc::channel::<String>(4);

    let login = Arc::new(steps::Login {
        mgr,
        mstore,
        identity_store,
        broker,
        session_id,
        provider_id,
        into_bundle_id,
        bundle_dir,
        direct_account,
        account_id,
        cli_path,
        auth_login_args,
        auth_check_args,
        auth_env,
    });

    // PTY branch: providers like OpenClaw whose auth subcommand
    // refuses to run when `isatty()==0`. Spawn via portable_pty so the
    // child sees an interactive terminal, feed PTY output through the
    // same record_line matcher the pipes path uses. Stdout/stderr are
    // merged into the single PTY stream — record_line handles both as
    // identical input.
    if requires_tty {
        steps::spawn_auth_cli_pty(login, stdin_tx, stdin_rx);
        return;
    }

    steps::spawn_piped(login, stdin_tx, stdin_rx);
}

/// How long a provider's auth-status check may take (`confirm_authenticated`).
const AUTH_CHECK_TIMEOUT_SECS: u64 = 30;

/// Run the provider's auth-check subcommand and return true if it
/// exits 0. Failure modes (binary missing, network error, etc.) are
/// all treated as "not authenticated" — the caller will then either
/// keep waiting (drain task loop) or transition to Failed (exit
/// fallback).
async fn confirm_authenticated(
    cli_path: &str,
    args: &[String],
    env: &std::collections::HashMap<String, String>,
) -> bool {
    use std::process::Stdio;
    // Through the shim parser (`make_cli_cmd`): a raw npm `.cmd` shim spawned
    // with piped stdio can drop its arguments or never run the CLI, which
    // would report a good login as failed. It also sets CREATE_NO_WINDOW, so
    // the probe, polled during an auth flow, flashes no console.
    let mut c = agentmux_common::make_cli_cmd(cli_path);
    // The third spawn of this same provider CLI in this file — the auth-check
    // poll driven from the OAuth drain loop. Same strict policy as the other
    // two. (ReAgent P0, round 4, on #3326.)
    crate::backend::pane_env::sanitize_external_command(&mut c);
    c.args(args)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // A status check that hangs counts as not logged in (and is killed on drop).
    match tokio::time::timeout(std::time::Duration::from_secs(AUTH_CHECK_TIMEOUT_SECS), c.status()).await {
        Ok(Ok(s)) => s.success(),
        _ => false,
    }
}

#[cfg(test)]
#[path = "identity_auth_spawn_tests.rs"]
mod tests;
