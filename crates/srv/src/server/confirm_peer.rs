// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where the process that sends a headless sign-in's confirm stands. The confirm is the person's own answer, so it is taken
//! only from a process that is found and is outside every agent's process tree. One from inside an agent's tree, one whose
//! process cannot be found (the lookup failed, or the peer is not on IPv4 loopback) and one that came through a proxy or a
//! tunnel (it carries forwarding headers) are refused, and the refusal says what works. Every confirm is recorded by
//! [`super::local_session`].

use std::net::SocketAddr;

use axum::extract::connect_info::{ConnectInfo, Connected};
use axum::serve::IncomingStream;

use super::local_session::ConfirmHow;
use super::AppState;

/// Both ends of a connection as the listener accepted it. The peer alone is not enough to find who holds the other end: the
/// server's own end is whichever listener took the connection, not always the web port.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Conn {
    pub peer: SocketAddr,
    /// The listener's address for this connection; `None` when the system did not say.
    pub local: Option<SocketAddr>,
}

impl Connected<IncomingStream<'_>> for Conn {
    fn connect_info(stream: IncomingStream<'_>) -> Self {
        Conn { peer: stream.remote_addr(), local: stream.local_addr().ok() }
    }
}

/// Make the plain peer address (`ConnectInfo<SocketAddr>`, which the other routes read) available beside [`Conn`].
pub(crate) fn with_plain_peer(router: axum::Router) -> axum::Router {
    router.layer(axum::middleware::from_fn(plain_peer))
}

async fn plain_peer(mut req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    if let Some(ConnectInfo(conn)) = req.extensions().get::<ConnectInfo<Conn>>().copied() {
        req.extensions_mut().insert(ConnectInfo(conn.peer));
    }
    next.run(req).await
}

/// What an agent that sent a confirm is told.
pub(crate) const REFUSED_TEXT: &str = "this confirm is the person's own answer to their sign-in: ask the person to answer it in their terminal";

/// What is told when the confirm's sender could not be found, or came through a proxy: it was not taken, and the person's own
/// terminal on this machine, or their own browser on the printed address and code, still works.
pub(crate) const UNKNOWN_TEXT: &str = "this confirm could not be tied to the person's own terminal on this machine, so it was not taken: the person runs `agentmux-srv open` in their own terminal here, or opens the printed address and code in their own browser";

/// The text of a confirm that was not taken.
pub(crate) fn refusal_text(how: &ConfirmHow) -> &'static str {
    match how {
        ConfirmHow::Refused(_) => REFUSED_TEXT,
        _ => UNKNOWN_TEXT,
    }
}

/// Whether the request came through a proxy or a tunnel, by the headers one adds. The real client sends none of them.
pub(crate) fn is_forwarded(headers: &axum::http::HeaderMap) -> bool {
    ["forwarded", "x-forwarded-for", "x-forwarded-host", "x-forwarded-proto", "x-real-ip", "via"].iter().any(|h| headers.contains_key(*h))
}

/// How the confirm that arrived on `conn` was sent. It reads the process table, so it runs off the async threads.
pub(crate) async fn classify(state: &AppState, conn: Conn, forwarded: bool) -> ConfirmHow {
    if forwarded {
        return ConfirmHow::Forwarded;
    }
    let state = state.clone();
    tokio::task::spawn_blocking(move || classify_now(&state, conn)).await.unwrap_or(ConfirmHow::Unknown)
}

fn classify_now(state: &AppState, conn: Conn) -> ConfirmHow {
    #[cfg(test)]
    if let Some(how) = hook::take() {
        return how;
    }
    let Some(server) = conn.local else { return ConfirmHow::Unknown };
    let Some(pid) = super::peer_process::owner_of(conn.peer, server) else { return ConfirmHow::Unknown };
    let Some(registry) = crate::backend::process_tracker::registry::global() else { return ConfirmHow::Unknown };
    let chain = with_ancestors(pid);
    match registry.agent_block_holding(&chain) {
        None => ConfirmHow::Outside,
        Some(block) => {
            let agent = crate::identity::resolver::block_agent_id(&state.mstore, &block).unwrap_or(block);
            ConfirmHow::Refused(format!("agent:{agent}"))
        }
    }
}

/// `pid` and its parents, nearest first, as the process table records them (a parent that started after its child is a
/// reused id and ends the chain).
pub(crate) fn with_ancestors(pid: u32) -> Vec<u32> {
    let mut chain = vec![pid];
    let Ok(all) = agentmux_procstats::snapshot() else { return chain };
    let mut current = all.iter().find(|p| p.pid == pid);
    while let Some(p) = current {
        let Some(parent) = p.ppid.and_then(|pp| all.iter().find(|q| q.pid == pp)) else { break };
        if !p.is_child_of(parent) || chain.contains(&parent.pid) || chain.len() >= 64 {
            break;
        }
        chain.push(parent.pid);
        current = Some(parent);
    }
    chain
}

/// What a test makes the next confirm's classification: no process lookup, no tracker.
#[cfg(test)]
pub(crate) mod hook {
    use super::ConfirmHow;
    use std::sync::Mutex;

    static NEXT: Mutex<Option<ConfirmHow>> = Mutex::new(None);

    pub(crate) fn set(how: ConfirmHow) {
        *NEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(how);
    }

    pub(super) fn take() -> Option<ConfirmHow> {
        NEXT.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}
