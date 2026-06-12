// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! ShellNodeRunner — spawns a shell command and streams output to the
//! frontend as `shell_chunk` WPS events scoped to the agent's block.
//!
//! Launched by `handle_shell_create` (server/mod.rs) via `tokio::spawn`.
//! The runner is fire-and-forget; the HTTP handler returns the `shell_id`
//! to the MCP caller immediately while output streams asynchronously.
//!
//! Phase 2 of SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md. Uses
//! `tokio::process::Command` with captured pipes (no PTY in this phase;
//! PTY support is a Phase 3 follow-up that requires portable-pty wiring
//! similar to the existing ShellController).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::backend::wps::{Broker, WaveEvent, EVENT_SHELL_CHUNK};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub struct ShellNodeRunner {
    pub shell_id: String,
    pub block_id: String,
    pub cmd: String,
    pub cwd: Option<String>,
    pub extra_env: HashMap<String, String>,
    pub broker: Arc<Broker>,
}

impl ShellNodeRunner {
    pub async fn run(self) {
        let shell_id = self.shell_id.clone();
        let block_id = self.block_id.clone();
        let broker = self.broker.clone();

        let mut child_cmd = if cfg!(windows) {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/C", &self.cmd]);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", &self.cmd]);
            c
        };

        if let Some(ref cwd) = self.cwd {
            child_cmd.current_dir(cwd);
        }
        for (k, v) in &self.extra_env {
            child_cmd.env(k, v);
        }

        child_cmd.stdout(std::process::Stdio::piped());
        child_cmd.stderr(std::process::Stdio::piped());

        let mut child = match child_cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                publish_chunk(&broker, &block_id, &shell_id, "system", &format!("[spawn error: {e}]"), now_ms());
                publish_exit(&broker, &block_id, &shell_id, 1, now_ms());
                return;
            }
        };

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        // Collect both stdout and stderr into a single ordered channel.
        // Each task owns its half of the pipe; the channel closes when both
        // tasks finish, at which point the main loop exits and we wait for exit.
        type Line = (&'static str, String, u64);
        let (tx, mut rx) = mpsc::unbounded_channel::<Line>();

        let tx_out = tx.clone();
        let t_stdout = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let ts = now_ms();
                let _ = tx_out.send(("stdout", line, ts));
            }
        });

        let tx_err = tx.clone();
        let t_stderr = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let ts = now_ms();
                let _ = tx_err.send(("stderr", line, ts));
            }
        });

        // Drop original sender so channel closes once both reader tasks finish.
        drop(tx);

        while let Some((kind, content, ts)) = rx.recv().await {
            publish_chunk(&broker, &block_id, &shell_id, kind, &content, ts);
        }

        let _ = tokio::join!(t_stdout, t_stderr);

        let exit_code = match child.wait().await {
            Ok(status) => status.code().unwrap_or(-1),
            Err(_) => -1,
        };

        publish_exit(&broker, &block_id, &shell_id, exit_code, now_ms());
    }
}

fn publish_chunk(broker: &Broker, block_id: &str, shell_id: &str, kind: &str, content: &str, ts: u64) {
    broker.publish(WaveEvent {
        event: EVENT_SHELL_CHUNK.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 1024,
        data: Some(serde_json::json!({
            "shell_id": shell_id,
            "op": "chunk",
            "kind": kind,
            "content": content,
            "timestamp": ts,
        })),
    });
}

fn publish_exit(broker: &Broker, block_id: &str, shell_id: &str, exit_code: i32, ts: u64) {
    broker.publish(WaveEvent {
        event: EVENT_SHELL_CHUNK.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 1,
        data: Some(serde_json::json!({
            "shell_id": shell_id,
            "op": "exit",
            "exit_code": exit_code,
            "timestamp": ts,
        })),
    });
}
