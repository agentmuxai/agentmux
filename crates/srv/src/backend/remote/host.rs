// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One SSH host reached for commands rather than a terminal: a durable pane's
//! `attach`, the helper install's scripts, and listing or ending the host's
//! durable sessions (SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md
//! §6.2, §7.6). The one place these run `ssh`.

use std::path::PathBuf;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use super::conn::SshDest;
use super::ConnTarget;

#[derive(Debug, Clone)]
pub struct HostSsh {
    pub dest: SshDest,
    pub ssh_path: PathBuf,
    /// ssh's connection-sharing directory (macOS and Linux), if any.
    pub control_dir: Option<PathBuf>,
    /// Set on `ssh` after the sanitizing: askpass ([`Self::ask_user_in`]).
    pub env: Vec<(String, String)>,
}

/// What a command on the host left behind.
#[derive(Debug)]
pub struct HostOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl HostOutput {
    /// Its output if it succeeded; otherwise ssh's or the command's last
    /// line of complaint.
    pub fn ok(self) -> Result<String, String> {
        if self.code == Some(0) {
            return Ok(self.stdout);
        }
        Err(self
            .stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("ssh failed")
            .trim()
            .to_string())
    }
}

impl HostSsh {
    /// The host of an SSH connection name, with this machine's `ssh`.
    pub fn for_connection(conn: &str) -> Result<Self, String> {
        let dest = match ConnTarget::parse(conn)? {
            ConnTarget::Ssh(d) => d,
            _ => return Err(format!("{conn:?} is not an SSH connection")),
        };
        let ssh_path = super::ssh::binary().ok_or_else(super::ssh::missing_binary_message)?;
        Ok(Self {
            dest,
            ssh_path,
            control_dir: super::ssh::control_dir(&crate::backend::base::get_mux_config_dir()),
            env: Vec::new(),
        })
    }

    /// Send ssh's prompts (a password, a passphrase, a new host key) to the
    /// user, in the window of their pane `block_id`, through the askpass
    /// bridge: this `ssh` has no terminal to ask in. The grant lasts until
    /// the returned guard is dropped. Nothing changes without the askpass
    /// helper; a login that needs a prompt then fails with ssh's reason.
    pub fn ask_user_in(
        &mut self,
        block_id: &str,
        conn: &str,
        auth_key: &str,
    ) -> Option<super::askpass::Revoke> {
        use super::askpass;
        let program = askpass::program()?;
        let secret = askpass::grant(askpass::AskpassGrant {
            agent_block_id: block_id.to_string(),
            agent: String::new(),
            connection: conn.to_string(),
            user_pane: true,
        });
        let local_url = std::env::var("AGENTMUX_LOCAL_URL").unwrap_or_default();
        self.env = askpass::ssh_env(&secret, &program, &local_url, auth_key);
        Some(askpass::Revoke(secret))
    }

    /// `ssh -T ... -- <host> <remote>`: no terminal on the remote side
    /// (frames or a script's output, not a session, go over it). The caller
    /// sets its stdio.
    pub fn command(&self, remote: &str) -> tokio::process::Command {
        let mut args = super::ssh::launch(&self.dest, remote, &[], "", self.control_dir.as_deref());
        args[0] = "-T".to_string();
        // With nowhere to ask (no askpass route), never prompt: on the
        // terminal srv was started from, if any, nobody would answer. ssh
        // fails with its reason instead.
        if self.env.is_empty() {
            args.insert(1, "-oBatchMode=yes".to_string());
        }
        let mut cmd = tokio::process::Command::new(&self.ssh_path);
        cmd.args(&args).kill_on_drop(true);
        #[cfg(windows)]
        {
            use agentmux_common::win32::NoWindow;
            cmd.no_window();
        }
        crate::backend::pane_env::sanitize_process_command(&mut cmd);
        // After the sanitizing: askpass needs srv's address and key.
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
        cmd
    }

    /// Run `remote` with `input` on its stdin, for at most `limit`, so a
    /// host that stops answering never holds the caller.
    pub async fn run(
        &self,
        remote: &str,
        input: Option<Vec<u8>>,
        limit: Duration,
    ) -> Result<HostOutput, String> {
        let mut cmd = self.command(remote);
        cmd.stdin(if input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("could not run ssh: {e}"))?;
        if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
            stdin
                .write_all(&bytes)
                .await
                .map_err(|e| format!("upload: {e}"))?;
            drop(stdin);
        }
        let out = tokio::time::timeout(limit, child.wait_with_output())
            .await
            .map_err(|_| "ssh took too long".to_string())?
            .map_err(|e| format!("ssh: {e}"))?;
        Ok(HostOutput {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}
