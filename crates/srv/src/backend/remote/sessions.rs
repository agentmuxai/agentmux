// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The durable sessions on an SSH host (spec §7.6): what the helper's daemon
//! holds there, which pane (if any) each belongs to, and ending one. Behind
//! the pane menu's "Sessions on <host>" and `muxsh conn sessions <host>`.

use std::time::Duration;

use serde::Serialize;

use super::host::HostSsh;
use crate::backend::blockcontroller::durable_ssh::{
    helper_path, valid_session_id, META_KEY_SESSION_ID,
};
use crate::backend::obj::{self, Block};
use crate::backend::storage::store::Store;

/// `ssh` exits 127 when the remote shell cannot find the command: no helper.
const EXIT_COMMAND_NOT_FOUND: i32 = 127;

/// How long listing or ending may take: a menu is waiting on it.
const LIMIT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostSession {
    /// The session id (`remote:session_id` of the pane that made it).
    pub id: String,
    /// How much output the session has produced, in bytes.
    pub bytes: u64,
    /// The shell's exit code, once it has exited (the daemon keeps an exited
    /// session for a few minutes so a reattach still sees how it ended).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exited: Option<i32>,
    /// The pane on this connection holding the session; none for an orphan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blockid: Option<String>,
}

/// The helper's `list`: one `id end exited` line per session, `exited` a code
/// or `-`. Lines that are not that are skipped, never guessed at.
pub fn parse_list(text: &str) -> Vec<HostSession> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let (id, end, exited) = (words.next()?, words.next()?, words.next()?);
            if words.next().is_some() || !valid_session_id(id) {
                return None;
            }
            let exited = match exited {
                "-" => None,
                code => Some(code.parse().ok()?),
            };
            Some(HostSession {
                id: id.to_string(),
                bytes: end.parse().ok()?,
                exited,
                blockid: None,
            })
        })
        .collect()
}

/// Mark each session with the pane on `conn` that holds it.
pub fn attribute(sessions: &mut [HostSession], blocks: &[Block], conn: &str) {
    for s in sessions.iter_mut() {
        s.blockid = blocks
            .iter()
            .find(|b| {
                obj::meta_get_string(&b.meta, META_KEY_SESSION_ID, "") == s.id
                    && super::conn::same_connection(
                        &obj::meta_get_string(
                            &b.meta,
                            crate::backend::blockcontroller::META_KEY_CONNECTION,
                            "",
                        ),
                        conn,
                    )
            })
            .map(|b| b.oid.clone());
    }
}

/// Where ssh's prompts go: the user's pane `block_id` (its window), with
/// srv's auth key for the askpass helper.
#[derive(Debug, Clone, Copy)]
pub struct AskIn<'a> {
    pub block_id: &'a str,
    pub auth_key: &'a str,
}

fn host_for(
    conn: &str,
    ask: Option<AskIn<'_>>,
) -> Result<(HostSsh, Option<super::askpass::Revoke>), String> {
    let mut host = HostSsh::for_connection(conn)?;
    let grant = ask.and_then(|a| host.ask_user_in(a.block_id, conn, a.auth_key));
    Ok((host, grant))
}

/// The sessions on `conn`'s host, each with its pane. A host without this
/// version's helper has none.
pub async fn list(
    store: &Store,
    conn: &str,
    ask: Option<AskIn<'_>>,
) -> Result<Vec<HostSession>, String> {
    let (host, _grant) = host_for(conn, ask)?;
    let out = host
        .run(&format!("{} list", helper_path()), None, LIMIT)
        .await?;
    if out.code == Some(EXIT_COMMAND_NOT_FOUND) {
        return Ok(Vec::new());
    }
    let mut sessions = parse_list(&out.ok()?);
    let blocks = store.get_all::<Block>().map_err(|e| e.to_string())?;
    attribute(&mut sessions, &blocks, conn);
    Ok(sessions)
}

/// End session `id` on `conn`'s host: its shell and everything it started.
/// `false` if the host had no such session.
pub async fn end(conn: &str, id: &str, ask: Option<AskIn<'_>>) -> Result<bool, String> {
    if !valid_session_id(id) {
        return Err(format!("{id:?} is not a session id"));
    }
    let (host, _grant) = host_for(conn, ask)?;
    let out = host
        .run(
            &format!("{} end --session {id}", helper_path()),
            None,
            LIMIT,
        )
        .await?;
    if out.code == Some(EXIT_COMMAND_NOT_FOUND) {
        return Ok(false);
    }
    Ok(out.ok()?.trim() == "ok")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_list_is_read_line_by_line() {
        let got =
            parse_list("amx-1 1024 -\namx-2 7 0\n\ngarbage\namx-3 x -\na;b 1 -\namx-4 1 - extra\n");
        assert_eq!(
            got,
            vec![
                HostSession {
                    id: "amx-1".into(),
                    bytes: 1024,
                    exited: None,
                    blockid: None
                },
                HostSession {
                    id: "amx-2".into(),
                    bytes: 7,
                    exited: Some(0),
                    blockid: None
                },
            ]
        );
    }

    #[test]
    fn a_session_belongs_to_the_pane_on_its_connection() {
        let block = |oid: &str, conn: &str, sid: &str| {
            let mut b = Block {
                oid: oid.into(),
                ..Default::default()
            };
            b.meta.insert("connection".into(), serde_json::json!(conn));
            b.meta
                .insert(META_KEY_SESSION_ID.into(), serde_json::json!(sid));
            b
        };
        let blocks = vec![
            block("b-other-host", "other", "amx-1"),
            block("b-here", "user@box", "amx-1"),
            block("b-none", "user@box", "amx-9"),
        ];
        let mut sessions = parse_list("amx-1 5 -\namx-2 5 -\n");
        attribute(&mut sessions, &blocks, "user@box");
        assert_eq!(sessions[0].blockid.as_deref(), Some("b-here"));
        // An orphan: no pane holds it.
        assert_eq!(sessions[1].blockid, None);
    }
}
