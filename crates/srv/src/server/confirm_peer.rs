// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where the process that sends a headless sign-in's confirm stands. The confirm is the person's own answer, so one sent by
//! a process inside an agent's process tree is refused, and the refusal tells the agent to ask the person. A confirm from any
//! other process goes through. When the process cannot be found (the lookup failed, or the peer is a proxy or a tunnel) the
//! confirm goes through too, and is recorded as `peer unknown`. Every confirm is recorded by [`super::local_session`].

use std::net::SocketAddr;

use super::local_session::ConfirmHow;
use super::AppState;

/// What an agent that sent a confirm is told.
pub(crate) const REFUSED_TEXT: &str = "this confirm is the person's own answer to their sign-in: ask the person to answer it in their terminal";

/// How the confirm sent from `peer` to the server at `server` arrived.
pub(crate) fn classify(state: &AppState, peer: SocketAddr, server: SocketAddr) -> ConfirmHow {
    #[cfg(test)]
    if let Some(how) = hook::take() {
        return how;
    }
    let Some(pid) = super::peer_process::owner_of(peer, server) else { return ConfirmHow::Unknown };
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
fn with_ancestors(pid: u32) -> Vec<u32> {
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
    use std::cell::RefCell;

    thread_local! {
        static NEXT: RefCell<Option<ConfirmHow>> = const { RefCell::new(None) };
    }

    pub(crate) fn set(how: ConfirmHow) {
        NEXT.with(|n| *n.borrow_mut() = Some(how));
    }

    pub(super) fn take() -> Option<ConfirmHow> {
        NEXT.with(|n| n.borrow_mut().take())
    }
}
