// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a message for an agent block goes. Moved down from `bootstrap/delivery.rs` unchanged: it reaches only
//! the controllers and the reactive layer, so the supervisor watcher (core) and the Swarm broadcast (server)
//! both route through it, and `bootstrap` re-exports it under the old path.

use crate::backend;
use crate::backend::reactive;

/// Where a message for an agent block goes.
pub enum AgentRoute {
    /// Taken on the controller's own channel — persistent stdin, ACP
    /// `session/prompt`, App Server — either now (`Delivered`) or queued for when
    /// the process can take it (`Deferred`). `Pty`: the controller is PTY-based,
    /// and the caller decides what that means for it.
    Sent(reactive::SenderDelivery),
    /// No structured channel takes it now: the caller starts a turn
    /// (`run_agent_turn`). `subprocess` is true for a `SubprocessController`,
    /// false for a persistent controller that is registered but not yet spawned.
    StartTurn { subprocess: bool },
}

/// The routing half of the reactive sender (`bootstrap/delivery.rs`), shared with the Swarm broadcast
/// (`server::app_api::fleet`) so a broadcast reaches ACP, App Server, running
/// persistent and not-yet-spawned agents exactly as an inter-agent message does.
///
/// Persistent + ACP + App Server keep their structured delivery; genuine PTY
/// controllers report `Pty`; a `SubprocessController` has no PTY and rejects raw
/// input, so it gets a turn instead (docs/reports/
/// REPORT_JEKT_DELIVERY_DROPS_SUBPROCESS_AGENTS_2026_09_02.md). An `Err` is a
/// structured controller that failed to take the message; it must not be retried
/// as keystrokes.
///
/// Saying who the input is from: the Swarm broadcast
/// labels its delivery to a running persistent agent as the user's
/// (`deliver_agent_message_from`). `None` is the plain text route the tests use.
pub fn route_agent_message_from(
    block_id: &str,
    message: &str,
    origin: Option<backend::blockcontroller::health::TurnInput>,
    from: backend::message_origin::MessageOrigin,
    inline: &[serde_json::Value],
    expires_at_ms: Option<i64>,
) -> Result<AgentRoute, String> {
    let Some(ctrl) = backend::blockcontroller::get_controller(block_id) else {
        return Err(format!("{} {block_id}", reactive::controller_wait::NO_CONTROLLER));
    };
    // A jekt, cron or broadcast is a real turn: it ends the hidden-reinjection
    // window as a person typing does, so ambient reads resume for an agent
    // that is only ever driven by messages.
    crate::ambient::digest::set_hidden_reinjection_active(block_id, false);
    let is_subprocess = ctrl
        .as_any()
        .downcast_ref::<backend::blockcontroller::subprocess::SubprocessController>()
        .is_some();
    if is_subprocess {
        return Ok(AgentRoute::StartTurn { subprocess: true });
    }
    match backend::blockcontroller::deliver_agent_message_from(block_id, message, origin, from, inline, expires_at_ms) {
        Ok(backend::blockcontroller::AgentDelivery::Structured) => {
            Ok(AgentRoute::Sent(reactive::SenderDelivery::Delivered))
        }
        Ok(backend::blockcontroller::AgentDelivery::StructuredDeferred) => {
            Ok(AgentRoute::Sent(reactive::SenderDelivery::Deferred))
        }
        Ok(backend::blockcontroller::AgentDelivery::Pty) => Ok(AgentRoute::Sent(reactive::SenderDelivery::Pty)),
        Err(e) => {
            // A persistent controller that is REGISTERED BUT NOT YET SPAWNED can't
            // be steered — `deliver_agent_message` writes to a live stdin and there
            // isn't one — but it can be STARTED. Controllers register lazily, so
            // after any srv restart every persistent agent sits in this state until
            // something is sent to it; without this, first contact fails with
            // "persistent process not running"
            // (docs/reports/REPORT_JEKT_DELIVERY_DROPS_UNSPAWNED_PERSISTENT_AGENTS_2026_09_03.md).
            //
            // Narrow on purpose: `needs_spawn()` is false while a spawn is already
            // in flight (that surfaces as the retryable "still starting up"), and
            // false for a live process whose delivery failed for another reason;
            // both keep returning the original error rather than start a second
            // turn.
            // An ACP or MSP agent whose process exited likewise: the turn
            // path starts it again (`SessionAgent::ensure_started`).
            let recoverable = ctrl
                .as_any()
                .downcast_ref::<backend::blockcontroller::persistent::PersistentSubprocessController>()
                .is_some_and(|p| p.needs_spawn())
                || backend::blockcontroller::session_agent::session_agent(ctrl.as_ref())
                    .is_some_and(|a| !a.is_running());
            if !recoverable {
                return Err(e);
            }
            // Nothing was persisted or written, so falling through cannot
            // double-deliver.
            tracing::info!(
                block_id = %block_id,
                error = %e,
                "reactive delivery: persistent controller not yet spawned — starting a turn instead"
            );
            Ok(AgentRoute::StartTurn { subprocess: false })
        }
    }
}
