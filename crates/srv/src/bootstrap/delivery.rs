// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Split out of bootstrap.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.3).

use super::*;

/// Deliver cron fires in process, through the server's shared inject path
/// (`server::reactive::deliver`) on the instance-key tier — identity M4c-3
/// (SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §6.5.9). The fire
/// keeps every tier the route has (this instance, cross-instance,
/// cross-channel, LAN, cloud) and audits the job's creator UID here only.
///
/// Must run AFTER `build_app_state` and BEFORE `cron_scheduler.start()`, so
/// no fire takes the HTTP fallback.
pub fn install_cron_delivery(state: &AppState) {
    let st = state.clone();
    state.cron_scheduler.install_delivery(Arc::new(move |req, creator_uid| {
        let st = st.clone();
        Box::pin(async move {
            crate::server::reactive::deliver(
                &st,
                crate::server::ReactiveAuthVia::FullAuthKey,
                &creator_uid,
                req,
            )
            .await
        })
    }));
}

/// Give the reactive handler a delivery route to `SubprocessController` agents.
///
/// `spawn_background_subsystems` installs a message sender that can only reach
/// the two controller kinds `deliver_agent_message` knows about — persistent
/// (stream-json stdin) and ACP (`session/prompt`). Everything else it reports as
/// [`AgentDelivery::Pty`], so the caller falls back to keystroke injection.
///
/// That fallback is wrong for a `SubprocessController`, which has no PTY and
/// rejects raw input outright ("subprocess controller does not accept raw input;
/// use AgentInputCommand"). The result was that EVERY inter-agent message to
/// such an agent was dropped: 6 of 10 providers as host agents (codex, gemini,
/// qwen, kimi, muxcode, antigravity) and every container agent of any provider,
/// since `agent_open.rs` forces `controller_type = "subprocess"` for container
/// mode. The swarm pane still listed them as present, because registration and
/// delivery are separate paths. See
/// `docs/reports/REPORT_JEKT_DELIVERY_DROPS_SUBPROCESS_AGENTS_2026_09_02.md`.
///
/// This re-installs the sender with the missing branch: a subprocess target gets
/// a real turn via `run_agent_turn` — the same thing `AgentInputCommand` does
/// when the operator types into the pane — instead of keystrokes it will refuse.
///
/// Must run AFTER `build_app_state`: unlike the early sender, the turn needs
/// `AppState` (stores, identity, broker, container manager) and is async.
pub fn install_agent_turn_delivery(state: &AppState) {
    let deps = crate::server::agent_handlers::AgentTurnDeps::from_state(state);
    state
        .reactive_handler
        .set_message_sender(Arc::new(move |block_id: &str, message: &str| {
            // Persistent + ACP keep their existing structured delivery, and
            // genuine PTY controllers (shell/term) keep falling back to
            // keystrokes. Only the subprocess case changes.
            let Some(ctrl) = backend::blockcontroller::get_controller(block_id) else {
                return Err(format!("no controller for block {block_id}"));
            };
            let is_subprocess = ctrl
                .as_any()
                .downcast_ref::<backend::blockcontroller::subprocess::SubprocessController>()
                .is_some();
            if !is_subprocess {
                match backend::blockcontroller::deliver_agent_message(block_id, message) {
                    Ok(backend::blockcontroller::AgentDelivery::Structured) => return Ok(reactive::SenderDelivery::Delivered),
                    Ok(backend::blockcontroller::AgentDelivery::StructuredDeferred) => {
                        return Ok(reactive::SenderDelivery::Deferred)
                    }
                    Ok(backend::blockcontroller::AgentDelivery::Pty) => return Ok(reactive::SenderDelivery::Pty),
                    Err(e) => {
                        // A persistent controller that is REGISTERED BUT NOT YET
                        // SPAWNED can't be steered — `deliver_agent_message`
                        // writes to a live stdin and there isn't one — but it can
                        // be STARTED. Controllers register lazily ("spawns on
                        // first message"), so after any srv restart every
                        // persistent agent sits in this state until a human sends
                        // it something from the UI. Without this fall-through,
                        // agent-to-agent delivery to such an agent fails
                        // permanently with "persistent process not running", and
                        // first contact is exactly the case that breaks.
                        // #2930 built the machinery to start a turn from here and
                        // scoped it to subprocess controllers; this widens it to
                        // the one other case that needs it.
                        // docs/reports/REPORT_JEKT_DELIVERY_DROPS_UNSPAWNED_PERSISTENT_AGENTS_2026_09_03.md
                        //
                        // Narrow on purpose. `needs_spawn()` is false while a
                        // spawn is already in flight (that surfaces as the
                        // retryable "still starting up"), and false for a live
                        // process whose delivery failed for some other reason —
                        // both must keep returning the original error rather than
                        // starting a second turn.
                        let recoverable = ctrl
                            .as_any()
                            .downcast_ref::<backend::blockcontroller::persistent::PersistentSubprocessController>()
                            .is_some_and(|p| p.needs_spawn());
                        if !recoverable {
                            return Err(e);
                        }
                        // Nothing was persisted or written: `inject_message`
                        // returns before its blockfile append, so falling through
                        // cannot double-deliver.
                        tracing::info!(
                            block_id = %block_id,
                            error = %e,
                            "reactive delivery: persistent controller not yet spawned — starting a turn instead"
                        );
                    }
                }
            }

            // `MessageSender` is synchronous but starting a turn is not, so this
            // waits for the turn to START and reports what actually happened.
            //
            // It must NOT spawn-and-return-`Ok(true)` optimistically (codex P1 on
            // PR #2930). `success` is not advisory: `cloud_subscriber` treats a
            // successful delivery as final and releases the claim back to pending
            // for retry ONLY when `!delivery.success`
            // (`muxbus/cloud_subscriber.rs`). Reporting success before the
            // fallible work ran would therefore turn every transient failure —
            // stale block meta, an oauth spawn-gate refusal, Docker down,
            // `ensure_running` failing — into a WAN message that is marked
            // delivered, never retried, and permanently lost. An honest `Err` here
            // is what lets that message come back.
            //
            // `block_in_place` yields this worker thread back to the runtime for
            // the duration, so other tasks keep running. The cost is real and
            // worth naming: `inject_message_inner` calls this while holding the
            // reactive `Handler` mutex, so a slow start (first-time container
            // image pull is the pathological case — everything else is a block
            // read plus an already-running `docker exec`) serializes other
            // injections behind it. Bounded stalling beats silent loss.
            //
            // `run_agent_turn` returns once the turn has been STARTED, not once
            // the agent has answered, so this is not waiting on model latency.
            //
            // `TurnRegistration::Skip` is LOAD-BEARING, not a tidy-up. We are
            // running under that same non-reentrant `Mutex<Handler>`, so the
            // registration tail's `get_global_handler().register_agent(...)`
            // would try to re-lock a mutex this very thread already holds and
            // wedge the reactive handler process-wide (reagent P0 on PR #2930).
            // It is also simply redundant here: this block was resolved BY an
            // `agent_to_block` lookup in that handler, so the agent is
            // registered by construction. Do not "simplify" this to Register.
            let Ok(handle) = tokio::runtime::Handle::try_current() else {
                return Err(
                    "no tokio runtime available to start an agent turn".to_string(),
                );
            };
            // `block_in_place` panics on a current-thread runtime. Production is
            // multi-thread (`#[tokio::main]`); refusing loudly on the other flavor
            // is correct, since the alternative is the optimistic lie above.
            if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread {
                return Err(
                    "cannot start an agent turn from a current-thread runtime"
                        .to_string(),
                );
            }
            let deps = deps.clone();
            let block_id_owned = block_id.to_string();
            let message = message.to_string();
            tracing::info!(
                block_id = %block_id_owned,
                kind = if is_subprocess { "subprocess" } else { "persistent (not yet spawned)" },
                "reactive delivery: starting agent turn (no PTY fallback)"
            );
            let started = tokio::task::block_in_place(|| {
                handle.block_on(crate::server::agent_handlers::run_agent_turn(
                    &deps,
                    block_id_owned.clone(),
                    message,
                    None,
                    crate::server::agent_handlers::TurnRegistration::Skip,
                    // Reactive delivery: a jekt, cron, nudge, broadcast, loop.
                    crate::backend::blockcontroller::health::TurnOrigin::Automated,
                    Vec::new(),
                ))
            });
            match started {
                Ok(()) => Ok(reactive::SenderDelivery::Delivered),
                Err(e) => {
                    tracing::error!(
                        block_id = %block_id_owned,
                        error = %e,
                        "reactive delivery: agent turn failed to start"
                    );
                    Err(e)
                }
            }
        }));
}

/// Wires `blockcontroller::close_on_exit` (see that function's own doc
/// comment) up to the real close action, now that `AppState` exists.
/// Mirrors `install_agent_turn_delivery` just above — same "backend code
/// needs a capability only available once `AppState` is built" shape,
/// solved the same way (a process-wide callback set once at boot).
///
/// The callback itself is deliberately synchronous (`Fn(String, String)`,
/// not `async fn`) — `shell/lifecycle.rs`'s wait/cleanup task calls it from
/// a plain (non-async) decision point after its own cleanup, and trait
/// objects can't hold an `async fn` without extra boxing machinery this
/// doesn't need. `tokio::spawn` here is what actually runs the (necessarily
/// async) `sagas::delete_block::run` call; the callback returns
/// immediately, fire-and-forget, matching `close_on_exit`'s own contract.
///
/// See `docs/specs/SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` §10 for why
/// this exists: a shell pane's controller has no code path today that
/// closes it when the shell process exits, which (combined with §7-9's
/// fix for the OTHER half of that bug) leaves an exited pane sitting inert
/// — technically correct, but indistinguishable from "hung" to a user.
pub fn install_close_on_exit_handler(state: &AppState) {
    let state = state.clone();
    backend::blockcontroller::set_close_on_exit_handler(Arc::new(move |tab_id: String, block_id: String| {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::sagas::delete_block::run(&state, tab_id.clone(), block_id.clone()).await {
                // Best-effort, same posture as every other close-on-exit
                // failure mode: the shell already exited and its PTY is
                // already gone either way, so there is nothing left to roll
                // back — a failed close just leaves the pane sitting inert,
                // the same as if close-on-exit weren't enabled for it at
                // all, not a lost or corrupted state.
                tracing::warn!(
                    tab_id = %tab_id,
                    block_id = %block_id,
                    error = %e,
                    "close-on-exit: delete_block saga failed"
                );
            }
        });
    }));
}
