// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which websocket connection may call which command, decided at dispatch.
//!
//! The instance key opens `/ws`, and a native agent has it in its environment, so holding it proves nothing
//! about who is calling. A command is the AgentMux window's alone unless `backend::rpc::action_access` lists it:
//!
//! - a connection the host proved is the window (`RpcContext::operator_proven_now`) calls anything;
//! - any connection calls an open command (`OPEN`: reads the window shows anyone who can open it, and the calls
//!   the handler itself decides per caller);
//! - a connection registered as an agent reaches only the open commands and the ones whose handler checks the
//!   pane (`OWN_PANE`); everything else is refused, whatever the agent's own rules say;
//! - any other connection is refused with the words the app matches to prove the connection again.
//!
//! A headless srv with no registered external host and no sign-in of its own has nothing that could vouch for a
//! connection, so it admits an unproven one and logs once per connection (the same fallback as the composer's
//! `agentinput`, `agent_handlers::composer_origin`); with either present the guard is strict.
//!
//! A dev build runs the guard like any other. `AGENTMUX_DEV_OPEN_RPC=1`, given when a dev instance is started, turns
//! it off for the harnesses in `tools/tests` that drive an unproven connection; any other mode ignores it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use crate::backend::rpc::action_access;
use crate::backend::rpc::engine::{DispatchGuard, WshRpcEngine};
use crate::backend::rpc_types::RpcContext;

/// The environment variable that turns the guard off, in a dev build only.
pub(crate) const DEV_SWITCH: &str = "AGENTMUX_DEV_OPEN_RPC";

/// What the decision reads besides the call: how this srv was started.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Mode {
    /// The dev switch was given and this is a dev build.
    pub dev_open: bool,
    /// A headless srv (no CEF host).
    pub headless: bool,
    /// A registered external host that attests connections.
    pub external_host: bool,
    /// A browser can sign in to this srv, or one has.
    pub local_sign_in: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Allow,
    /// Allowed because nothing could prove the caller (headless, no host, no sign-in): the caller logs it.
    AllowUnproven,
    Refuse(String),
}

pub(crate) fn decide(command: &str, ctx: &RpcContext, mode: Mode) -> Verdict {
    if mode.dev_open || action_access::is_open(command) {
        return Verdict::Allow;
    }
    if !ctx.agent_id.is_empty() {
        return if action_access::agent_reaches_handler(command) {
            Verdict::Allow
        } else {
            Verdict::Refuse(format!("FORBIDDEN: {command} is the AgentMux window's call; an agent reaches AgentMux with its MuxAPI tools"))
        };
    }
    if ctx.operator_proven_now() {
        return Verdict::Allow;
    }
    if mode.headless && !mode.external_host && !mode.local_sign_in && !ctx.operator_proven {
        return Verdict::AllowUnproven;
    }
    match ctx.operator_only(&format!("{command} is the AgentMux window's call")) {
        Err(refusal) => Verdict::Refuse(refusal),
        Ok(()) => Verdict::Allow,
    }
}

/// How this srv was started, read once.
fn mode() -> Mode {
    // The srv's own tests drive commands over sockets that are not the window (the router's tests, the live ones); the
    // guard is tested with explicit modes, through `decide` and `guard_with`.
    #[cfg(test)]
    return Mode { dev_open: true, ..Mode::default() };
    #[cfg(not(test))]
    live_mode()
}

#[cfg(not(test))]
fn live_mode() -> Mode {
    static DEV_OPEN: OnceLock<bool> = OnceLock::new();
    let dev_open = *DEV_OPEN.get_or_init(|| {
        dev_open_from(agentmux_common::RuntimeMode::from_env(), std::env::var(DEV_SWITCH).ok().as_deref())
    });
    Mode {
        dev_open,
        headless: crate::headless::active(),
        external_host: super::external_host::host().is_some(),
        local_sign_in: super::local_session_routes::sign_in_exists(),
    }
}

/// The switch counts only in a dev build.
pub(crate) fn dev_open_from(runtime: Option<agentmux_common::RuntimeMode>, var: Option<&str>) -> bool {
    matches!(runtime, Some(agentmux_common::RuntimeMode::Dev { .. })) && matches!(var, Some("1") | Some("true"))
}

/// Guard `engine`'s connection. The unproven-admission is logged once for this connection.
pub(crate) fn install(engine: &Arc<WshRpcEngine>) {
    engine.set_dispatch_guard(guard_with(mode));
}

fn guard_with(mode: impl Fn() -> Mode + Send + Sync + 'static) -> DispatchGuard {
    let logged = AtomicBool::new(false);
    Arc::new(move |command: &str, ctx: &RpcContext| {
        // The window's own calls, keystrokes among them, take no further look.
        if ctx.agent_id.is_empty() && ctx.operator_proven_now() {
            return Ok(());
        }
        match decide(command, ctx, mode()) {
            Verdict::Allow => Ok(()),
            Verdict::AllowUnproven => {
                if !logged.swap(true, Ordering::Relaxed) {
                    tracing::warn!(client_type = %ctx.client_type, "a connection no host proved calls the window's commands, in a headless srv with no host to prove it");
                }
                Ok(())
            }
            Verdict::Refuse(refusal) => Err(refusal),
        }
    })
}

#[cfg(test)]
#[path = "tests/dispatch_guard_tests.rs"]
mod dispatch_guard_tests;
