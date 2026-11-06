// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `google.status` and `google.grant` (docs/specs/SPEC_GOOGLE_PHASE_B_AGENTMUXS_OWN_CLIENT_2026_11_01.md §7):
//! which Google accounts are connected here and who holds what, and turning a service of one on or off
//! for an agent on this machine. Both run the code the `Google` tool's `status` and `grant` run; a grant
//! is recorded as the remote agent, `slug@machine`, in the account's log and the window notice. `connect`
//! has no remote form: the sign-in is a person's, at the machine that holds the account.

use serde_json::Value;

use super::{AppState, RemoteCaller, RemoteRefusal, RemoteResult};
use crate::server::google_handlers::{grant_as, status_for, GrantBody};

/// The connected accounts, what each supports and who holds what. A remote caller has no agent here, so
/// there is no `yours`.
pub(super) async fn status(state: &AppState, _caller: &RemoteCaller, _params: Value) -> RemoteResult {
    status_for(state, None).map_err(RemoteRefusal::failed)
}

/// Turn a service of an account on (`level` read or write) or off (`off`) for an agent here. A state
/// change: the transport has already required a certificate from a trusted install for it.
pub(super) async fn grant(state: &AppState, caller: &RemoteCaller, params: Value) -> RemoteResult {
    let names_an_agent = params.get("agent").and_then(Value::as_str).is_some_and(|a| !a.trim().is_empty());
    if !names_an_agent {
        return Err(RemoteRefusal::new("bad_params", "google.grant: name the agent on this machine (agent): there is no \"yourself\" across machines"));
    }
    let body: GrantBody = serde_json::from_value(params).map_err(|e| RemoteRefusal::new("bad_params", format!("google.grant: {e}")))?;
    let actor = caller.actor();
    let done = grant_as(state, &actor, &actor, None, &body).map_err(RemoteRefusal::failed)?;
    tracing::info!(actor = %actor, "remote google.grant: a grant was set");
    Ok(done)
}

#[cfg(test)]
#[path = "google_ops_tests.rs"]
mod tests;
