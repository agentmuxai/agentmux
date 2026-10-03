// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The SSH askpass bridge (spec §5.3 of
//! SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md).
//!
//! When an agent runs something over SSH there is no terminal for `ssh` to ask
//! a password, a passphrase or a new host key in, and the agent must never see
//! or answer one. So that `ssh` runs with `SSH_ASKPASS` pointing at
//! `agentmux-bashwrap` and `SSH_ASKPASS_REQUIRE=force`: every prompt goes to
//! bashwrap, which asks srv, which shows it to the user in a dialog, and the
//! answer goes straight back to `ssh`.
//!
//! Who may ask is the security question: every agent holds srv's auth key, so
//! a route open to that key alone would let an agent pop a fake "password"
//! dialog and read what the user types. Each such `ssh` therefore gets its own
//! secret, in its own environment and nowhere else; the askpass route answers
//! only a live secret, and the secret decides whose pane the dialog names. It
//! is revoked when that `ssh` exits.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};

/// Where bashwrap finds the secret; set only on the `ssh` it belongs to.
pub const ENV_SECRET: &str = "AGENTMUX_ASKPASS_SECRET";

/// Block meta on an agent's SSH PtyShell: the agent's pane and name. Its `ssh`
/// then gets an askpass secret (`blockcontroller::shell::lifecycle`), so its
/// prompts go to the user, never to the terminal the agent reads.
pub const META_KEY_AGENT_BLOCK: &str = "ptyshell:askpassagentblock";
pub const META_KEY_AGENT: &str = "ptyshell:askpassagent";

/// What a secret was issued for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskpassGrant {
    /// The agent's pane, whose window shows the dialog.
    pub agent_block_id: String,
    /// The agent, named in the dialog.
    pub agent: String,
    /// The connection `ssh` is opening, named in the dialog.
    pub connection: String,
}

fn registry() -> &'static Mutex<HashMap<[u8; 32], AskpassGrant>> {
    static REG: OnceLock<Mutex<HashMap<[u8; 32], AskpassGrant>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Stored by digest, so a timing difference in the lookup says nothing about
/// how much of a guessed secret matches a real one.
fn digest(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

/// Issue a secret for one `ssh` run.
pub fn grant(grant: AskpassGrant) -> String {
    let secret = format!(
        "amxa_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(digest(&secret), grant);
    secret
}

/// What `secret` was issued for, while its `ssh` runs.
pub fn lookup(secret: &str) -> Option<AskpassGrant> {
    if secret.is_empty() {
        return None;
    }
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&digest(secret))
        .cloned()
}

/// End a secret: its `ssh` exited.
pub fn revoke(secret: &str) {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&digest(secret));
}

/// Revokes its secret when dropped: held by whatever waits for the `ssh`, so
/// every way that `ssh` ends (exit, failed spawn, stop) ends the secret too.
pub struct Revoke(pub String);

impl Drop for Revoke {
    fn drop(&mut self) {
        revoke(&self.0);
    }
}

/// How a prompt is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// A question answered yes or no: a new host key ("Are you sure you want
    /// to continue connecting (yes/no/[fingerprint])?"), or confirming a key's
    /// use. `ssh` takes the word typed.
    YesNo,
    /// A password, passphrase or one-time code: hidden, never logged.
    Secret,
}

/// What kind of answer `prompt` wants.
pub fn classify(prompt: &str) -> PromptKind {
    let p = prompt.to_ascii_lowercase();
    if p.contains("(yes/no") || p.contains("[yes/no") {
        PromptKind::YesNo
    } else {
        PromptKind::Secret
    }
}

/// The environment the `ssh` gets: askpass for every prompt, with its secret,
/// and how bashwrap reaches srv.
pub fn ssh_env(
    secret: &str,
    program: &std::path::Path,
    local_url: &str,
    auth_key: &str,
) -> Vec<(String, String)> {
    vec![
        ("SSH_ASKPASS".into(), program.to_string_lossy().into_owned()),
        // Askpass even where a terminal exists (OpenSSH 8.4+, every platform
        // AgentMux supports), so nothing ever prompts where an agent reads.
        ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
        // Older ssh on Linux uses askpass only with a display set.
        (
            "DISPLAY".into(),
            std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into()),
        ),
        (ENV_SECRET.into(), secret.into()),
        ("AGENTMUX_LOCAL_URL".into(), local_url.into()),
        ("AGENTMUX_AUTH_KEY".into(), auth_key.into()),
    ]
}

/// The askpass program, `agentmux-bashwrap`: the bundled one, or (a dev build)
/// the one beside srv, or the first on `PATH`.
pub fn program() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "agentmux-bashwrap.exe"
    } else {
        "agentmux-bashwrap"
    };
    let beside_srv = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)));
    crate::backend::tool_store::bundled_tools_dir()
        .map(|d| d.join(name))
        .into_iter()
        .chain(beside_srv)
        .find(|p| p.is_file())
        .or_else(|| which::which("agentmux-bashwrap").ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(conn: &str) -> AskpassGrant {
        AskpassGrant {
            agent_block_id: "blk".into(),
            agent: "korp".into(),
            connection: conn.into(),
        }
    }

    #[test]
    fn a_secret_answers_while_its_ssh_runs_and_not_after() {
        let secret = grant(g("area54"));
        assert!(secret.starts_with("amxa_") && secret.len() > 60);
        assert_eq!(lookup(&secret), Some(g("area54")));
        let other = grant(g("nas"));
        assert_ne!(secret, other, "one secret per run");
        revoke(&secret);
        assert_eq!(lookup(&secret), None);
        assert_eq!(
            lookup(&other),
            Some(g("nas")),
            "revoking one leaves the others"
        );
        assert_eq!(lookup(""), None);
        assert_eq!(lookup("amxa_guess"), None);
        drop(Revoke(other.clone()));
        assert_eq!(
            lookup(&other),
            None,
            "the guard revokes when its ssh's waiter ends"
        );
    }

    #[test]
    fn a_host_key_question_is_yes_or_no_and_everything_else_is_secret() {
        assert_eq!(
            classify("Are you sure you want to continue connecting (yes/no/[fingerprint])? "),
            PromptKind::YesNo
        );
        assert_eq!(
            classify("Allow use of key id_ed25519? [yes/no]"),
            PromptKind::YesNo
        );
        assert_eq!(classify("asaf@area54's password: "), PromptKind::Secret);
        assert_eq!(
            classify("Enter passphrase for key '/home/u/.ssh/id_ed25519': "),
            PromptKind::Secret
        );
        assert_eq!(classify("Verification code: "), PromptKind::Secret);
    }

    #[test]
    fn the_ssh_env_forces_askpass_and_carries_only_its_secret() {
        let env = ssh_env(
            "amxa_x",
            std::path::Path::new("/opt/agentmux-bashwrap"),
            "http://127.0.0.1:1",
            "k",
        );
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("SSH_ASKPASS"), Some("/opt/agentmux-bashwrap"));
        assert_eq!(get("SSH_ASKPASS_REQUIRE"), Some("force"));
        assert_eq!(get(ENV_SECRET), Some("amxa_x"));
        assert!(get("DISPLAY").is_some());
    }
}
