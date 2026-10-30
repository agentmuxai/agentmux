// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Remote operations: an agent does on another proven machine what it can do on its own
//! (`docs/specs/SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md`).
//!
//! The transport (this module's `receive` and `send`) owns the signature, the audience,
//! the clock window, the nonce, the machine's proof, the off switch and both audit rows.
//! An operation is a handler that gets a [`RemoteCaller`] and its `params` and never sees
//! the envelope: it runs the same code a local call runs, with [`RemoteCaller::actor`] as
//! the actor, and anything that is sensitive locally files the same sensitive request.
//!
//! To add an operation: write `async fn` in a file of its own, and add one arm to [`lookup`].

pub(crate) mod app_ops;
pub(crate) mod fleet_sync;
pub(crate) mod identity_ops;
pub(crate) mod memory;
pub(crate) mod receive;
pub(crate) mod send;

use futures_util::future::BoxFuture;
use serde_json::Value;

use super::AppState;

/// How the requesting machine was proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verification {
    /// A LAN peer whose agent key this machine pinned, signing a request meant for it.
    Lan,
    /// An approved install of the same account, over the relay.
    #[allow(dead_code)] // the relay kind follows the LAN transport
    Wan,
}

impl Verification {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Verification::Lan => "lan-verified",
            Verification::Wan => "wan-verified",
        }
    }
}

/// Who asks, as the receiving machine proved it.
///
/// The identity is the **slug**: the key this machine pinned for that name signed the request.
/// `agent_uid` and `reason` are signed too, but nothing ties the uid to the key, so a handler
/// uses them to describe the caller and never to authorize it. `instance_id` is the install
/// whose certificate vouches for the agent's key; it is empty when no valid certificate is held.
/// `install_trusted` says this machine also trusts that install (pinned here, or live in the
/// account's directory, and not revoked): it is looked into only for an operation that changes
/// state, so it is `false` for a read, and always `true` inside a write's handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteCaller {
    pub agent_uid: String,
    pub slug: String,
    /// The requesting machine's name.
    pub machine: String,
    /// The requesting install's id, from the agent's certificate; empty when none was shown.
    pub instance_id: String,
    pub install_trusted: bool,
    pub verification: Verification,
    /// The requester's own words, to show as theirs. Never acted on, never given to an agent.
    pub reason: String,
}

impl RemoteCaller {
    /// `slug@machine`: the actor for audit rows, cards and history. Never the operator.
    pub(crate) fn actor(&self) -> String {
        format!("{}@{}", self.slug, self.machine)
    }
}

/// An operation that did not run, and why. `code` is snake_case and goes to the requester as
/// `refused_reason`; `message` is for the requester's agent to read. A plain failure uses `failed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteRefusal {
    pub code: &'static str,
    pub message: String,
}

impl RemoteRefusal {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub(crate) fn failed(message: impl Into<String>) -> Self {
        Self::new("failed", message)
    }
}

/// `Ok` is the operation's `result` (status `done`).
pub(crate) type RemoteResult = Result<Value, RemoteRefusal>;

pub(crate) type OpFn = for<'a> fn(&'a AppState, &'a RemoteCaller, Value) -> BoxFuture<'a, RemoteResult>;

/// One operation: its handler, and whether it changes anything on this machine.
#[derive(Clone, Copy)]
pub(crate) struct Op {
    pub run: OpFn,
    /// A state-changing operation needs a caller backed by a certificate from the install that
    /// hosts the agent, an install this machine trusts (`needs_certificate`, `install_not_trusted`
    /// otherwise). A read can go on the first-seen pin alone:
    /// that pin comes from unauthenticated discovery, which is enough to read a version and too
    /// thin to stop an agent or move an account.
    pub changes_state: bool,
    /// The most bytes of params this operation takes. The default is the transport's
    /// `MAX_PARAMS_BYTES`; one that carries text raises it with [`Op::with_max_params`]. The sender
    /// and the receiver both read it from here, the receiver before it verifies anything, so the
    /// unsigned op name only bounds the work.
    pub max_params: usize,
}

impl Op {
    pub(crate) const fn read(run: OpFn) -> Self {
        Self { run, changes_state: false, max_params: agentmux_common::remote_op::MAX_PARAMS_BYTES }
    }

    pub(crate) const fn write(run: OpFn) -> Self {
        Self { run, changes_state: true, max_params: agentmux_common::remote_op::MAX_PARAMS_BYTES }
    }

    pub(crate) const fn with_max_params(mut self, max_params: usize) -> Self {
        self.max_params = max_params;
        self
    }
}

/// The params limit of `op`: its own, or the default for one this build does not know.
pub(crate) fn max_params_of(op: &str) -> usize {
    lookup(op).map_or(agentmux_common::remote_op::MAX_PARAMS_BYTES, |o| o.max_params)
}

/// The operation `op`, if this build has one. One arm per operation: say whether it reads or writes.
pub(crate) fn lookup(op: &str) -> Option<Op> {
    Some(match op {
        "app.version" => Op::read(|state, caller, params| Box::pin(app_ops::version(state, caller, params))),
        "app.health" => Op::read(|state, caller, params| Box::pin(app_ops::health(state, caller, params))),
        // The signed-in accounts here with quota, and moving an agent to one: a switch changes
        // state, so it needs a certificate-backed caller.
        "identity.accounts" => Op::read(|state, caller, params| Box::pin(identity_ops::accounts(state, caller, params))),
        "identity.switch" => Op::write(|state, caller, params| Box::pin(identity_ops::switch(state, caller, params))),
        // A Global Memory entry is up to 1 MiB of text (the cap `globalmemory.write` has), so this one
        // takes 1.2 MB of params; every other operation keeps the default.
        "memory.global.put" => Op::write(|state, caller, params| Box::pin(memory::put(state, caller, params))).with_max_params(memory::MAX_PUT_PARAMS),
        "memory.global.remove" => Op::write(|state, caller, params| Box::pin(memory::remove(state, caller, params))),
        "memory.global.list" => Op::read(|state, caller, params| Box::pin(memory::list(state, caller, params))),
        #[cfg(test)]
        "test.big" => Op::read(|_, _, params| Box::pin(async move { Ok(serde_json::json!({ "bytes": params.to_string().len() })) })).with_max_params(1_200_000),
        #[cfg(test)]
        "test.change" => Op::write(|_, caller, _| Box::pin(async move { Ok(serde_json::json!({ "changed_by": caller.actor() })) })),
        _ => return None,
    })
}

/// Whether this machine takes remote operations: on unless `network:remote_ops` is `false`.
pub(crate) fn enabled(state: &AppState) -> bool {
    state.config_watcher.get_settings().network_remote_ops != Some(false)
}

#[cfg(test)]
mod tests;
