// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A request nobody reviewed (`SPEC_SUPER_AGENT_2026_10_25.md` §6). The request still gets its
//! countdown; only the agent in front is missing. When no super agent holds the seat or runs
//! here for a few seconds, or the one that does hasn't answered in two minutes, the countdown
//! starts anyway with twice the usual window (at most the longest), the banner says no super
//! agent reviewed it, and the person's veto is the only check.

// The watcher is started only outside tests, so the constants and `watch` look unused there.
#![cfg_attr(test, allow(dead_code))]

use std::time::Duration;

use crate::backend::sensitive_ops;
use crate::backend::sensitive_requests::{self as requests, Status};
use crate::sagas::sensitive_window as window;
use crate::server::AppState;
use agentmux_common::time::now_ms;

/// How long a super agent has to answer before the request goes ahead without it.
const ASK_WAIT: Duration = Duration::from_secs(120);
/// How long there may be no super agent running here before it does. Covers a restart.
const ABSENT_GRACE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub struct Timing {
    pub poll: Duration,
    pub absent_grace: Duration,
    pub ask_wait: Duration,
}

pub const TIMING: Timing = Timing { poll: POLL, absent_grace: ABSENT_GRACE, ask_wait: ASK_WAIT };

/// Watch a request just filed, and send it to the countdown if no super agent reviews it.
/// Tests drive `watch_with` directly: a watcher started by every filed request would answer
/// the ones that test the super agent's side.
pub fn watch(state: &AppState, request_id: &str) {
    #[cfg(not(test))]
    {
        let (state, id) = (state.clone(), request_id.to_string());
        tokio::spawn(async move { watch_with(&state, &id, TIMING, reviewer_here).await });
    }
    #[cfg(test)]
    let _ = (state, request_id);
}

/// Whether a super agent holds the seat and runs in this AgentMux.
fn reviewer_here(state: &AppState) -> bool {
    let defs = state.mstore.agent_def_list().unwrap_or_default();
    crate::backend::agent_seat::super_holder(&state.mstore, &defs).is_some_and(|uid| crate::backend::reactive::get_global_handler().has_uid_registration(&uid))
}

/// Wait until the request is answered, or no one is left to answer it, then act.
/// `reviewer` says whether a super agent is there to answer.
pub async fn watch_with(state: &AppState, request_id: &str, timing: Timing, reviewer: impl Fn(&AppState) -> bool) {
    let started = tokio::time::Instant::now();
    let mut absent_since: Option<tokio::time::Instant> = None;
    loop {
        match requests::registry().get(request_id) {
            Some(r) if r.status == Status::Asked => {}
            _ => return,
        }
        if reviewer(state) {
            absent_since = None;
        } else {
            absent_since.get_or_insert_with(tokio::time::Instant::now);
        }
        let absent = absent_since.is_some_and(|t| t.elapsed() >= timing.absent_grace);
        if absent || started.elapsed() >= timing.ask_wait {
            go_ahead(state, request_id, absent);
            return;
        }
        tokio::time::sleep(timing.poll).await;
    }
}

/// The countdown for a request no super agent reviewed, or its end when nothing can run it.
fn go_ahead(state: &AppState, request_id: &str, no_reviewer: bool) {
    let Some(request) = requests::registry().get(request_id) else { return };
    let Some(spec) = sensitive_ops::spec(&request.op) else { return };
    let why = if no_reviewer { "no super agent is running" } else { "the super agent didn't answer in time" };
    if !window::has_runner(&request.op) {
        // Nothing here can do it: say so, rather than count down to nothing.
        if requests::registry().finish(request_id, Status::Expired, now_ms()).is_ok() {
            window::audit_step(state, "no super agent", &request, "sensitive.fallback", false, Some("nothing can run it from here"), "expired");
            window::tell_askers(&request, &format!("[AgentMux] Nobody reviewed your request to {} ({why}), and this AgentMux can't run it, so it ended.", request.summary));
        }
        return;
    }
    // Answered meanwhile: the super agent's answer stands.
    let Ok(request) = requests::registry().fallback(request_id, now_ms()) else { return };
    let window_secs = spec.fallback_window_secs();
    if let Err(e) = window::open(state, request_id, Duration::from_secs(window_secs)) {
        tracing::warn!(request_id, error = %e, "sensitive request: the fallback countdown did not start");
        return;
    }
    window::audit_step(state, "no super agent", &request, "sensitive.fallback", true, None, &format!("countdown {window_secs} s, no super agent reviewed it"));
    window::tell_askers(
        &request,
        &format!("[AgentMux] Nobody reviewed your request to {} ({why}). The person has {window_secs} s to veto it; after that it happens.", request.summary),
    );
}

#[cfg(test)]
#[path = "sensitive_fallback_tests.rs"]
mod tests;
