// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How a plain SSH pane ends (remote terminals spec §5.2; #4513).
//!
//! A pane whose `ssh` had no connection, or lost it, is not a shell the user
//! ended: it stays, with a line under ssh's own reason, instead of closing as
//! an `exit` does; Enter starts ssh again (a finished pane's restart). srv
//! stopping the pane itself (close, restart) makes ssh exit 255 too, and is
//! not a lost connection.

use std::path::Path;
use std::sync::Arc;

use portable_pty::CommandBuilder;

use super::file_ops::handle_append_block_file;
use crate::backend::mps::Broker;
use crate::backend::remote::conn::SshDest;
use crate::backend::remote::{askpass, link, ssh};
use crate::backend::storage::filestore::FileStore;

/// Pane `block_id`'s `ssh` (`ssh_path`) to `dest`, running `cmd`
/// (`cmd_args`) in `cwd` there, sharing the channel's connection to it.
/// `askpass_secret`: an agent's SSH shell, whose every ssh prompt goes to the
/// user through the askpass bridge, never into this terminal, which the agent
/// reads.
pub(super) fn command(
    ssh_path: &Path,
    dest: &SshDest,
    (cmd, cmd_args, cwd): (&str, &[String], &str),
    askpass_secret: Option<&str>,
    auth_key: &str,
    block_id: &str,
) -> CommandBuilder {
    let mut args = ssh::launch(dest, cmd, cmd_args, cwd, ssh::shared_control_dir().as_deref());
    ssh::set_pane_env(&mut args, block_id);
    tracing::info!(dest = %dest.destination, args = ?args, "ssh spawn path");
    let mut c = CommandBuilder::new(ssh_path);
    c.args(args.iter().map(String::as_str));
    c.env("TERM", "xterm-256color");
    c.env("COLORTERM", "truecolor");
    // Here as well as on every pane's command in lifecycle.rs: it is
    // idempotent, and askpass's variables are on its keep list.
    crate::backend::pane_env::sanitize_pty_command(&mut c);
    if let (Some(secret), Some(program)) = (askpass_secret, askpass::program()) {
        let local_url = std::env::var("AGENTMUX_LOCAL_URL").unwrap_or_default();
        for (k, v) in askpass::ssh_env(secret, &program, &local_url, auth_key) {
            c.env(k, v);
        }
    }
    c
}

/// An SSH pane's `ssh` (to `dest`, on connection `conn`) ended with
/// `raw_exit_code`. Tells the connection, and returns the line the pane shows
/// when it stays because the connection is gone. `stopped_by_srv`: srv ended
/// it itself (`stop()` and `stop_for_replace()` set the status before they
/// signal the process).
pub(super) fn ended(conn: &str, dest: &str, raw_exit_code: Option<u32>, stopped_by_srv: bool) -> Option<String> {
    let lost = ssh::pane_lost_connection(raw_exit_code, stopped_by_srv);
    let note = lost.then(|| {
        if link::held(conn) {
            link::disconnected_note(conn)
        } else {
            ssh::lost_note(conn)
        }
    });
    link::pane_ended(conn, lost.then(|| ssh::lost_message(dest)).as_deref());
    note
}

/// Put `text` in pane `block_id`'s output (`pane_note`), after ssh's own last
/// words: the caller has let the output drain first.
pub(super) async fn write_note(broker: Arc<Broker>, filestore: Option<Arc<FileStore>>, block_id: String, text: String) {
    let _ = tokio::task::spawn_blocking(move || {
        let note = super::super::pane_note(&text);
        handle_append_block_file(&broker, &block_id, "term", &note, filestore.as_ref(), None)
    })
    .await;
}
