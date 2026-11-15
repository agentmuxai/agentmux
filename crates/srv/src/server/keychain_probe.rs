// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether macOS has a login keychain the Claude CLI can store a sign-in in.
//!
//! The CLI keeps its credential in the user's default keychain. Under a HOME with no login keychain (the app
//! started with HOME pointed at a scratch folder, or an ssh-only or service account) macOS opens its own "Keychain
//! Not Found" dialog and the CLI waits on it, so a sign-in the person finished reads as "timed out". This asks the
//! system with `security default-keychain`, which only reads: it never creates, resets or changes a keychain.

// Only macOS asks; the check itself is plain logic that every platform tests.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

/// The problem code the sign-in panel turns into words.
pub(crate) const NO_DEFAULT_KEYCHAIN: &str = "no-default-keychain";

/// The problem with the keychain `security default-keychain` reported, if it is one. A default keychain is
/// printed as a quoted path; one that was reported but is not on disk is as unusable as none. An answer that
/// cannot be read is no verdict: nothing is blocked on a guess.
pub(crate) fn keychain_problem(
    exited_ok: bool,
    stdout: &str,
    exists: impl Fn(&std::path::Path) -> bool,
) -> Option<&'static str> {
    if !exited_ok {
        return Some(NO_DEFAULT_KEYCHAIN);
    }
    let path = stdout.split('"').nth(1)?;
    if path.is_empty() || exists(std::path::Path::new(path)) {
        return None;
    }
    Some(NO_DEFAULT_KEYCHAIN)
}

/// The auth-check answer for an account whose sign-in cannot be kept, or None when the keychain is usable.
#[cfg(target_os = "macos")]
pub(crate) async fn no_keychain_result() -> Option<crate::backend::rpc_types::CheckCliAuthResult> {
    let problem = credential_store_problem().await?;
    tracing::info!(problem, "claude auth check: no usable keychain (macOS)");
    Some(crate::backend::rpc_types::CheckCliAuthResult {
        authenticated: false,
        email: None,
        auth_method: None,
        raw_output: "no usable keychain".to_string(),
        store_problem: Some(problem.to_string()),
    })
}

/// The keychain problem for the user this process runs as, or None (usable, or no answer in time).
#[cfg(target_os = "macos")]
pub(crate) async fn credential_store_problem() -> Option<&'static str> {
    let mut cmd = tokio::process::Command::new("/usr/bin/security");
    cmd.args(["default-keychain", "-d", "user"])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(3), cmd.output())
        .await
        .ok()?
        .ok()?;
    keychain_problem(
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        |p| p.exists(),
    )
}

#[cfg(test)]
#[path = "keychain_probe_tests.rs"]
mod tests;
