// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Supervisor's rules, with no I/O: which turns are judged, the limits,
//! and what each verdict leads to. Spec §3.2 and §3.4.

use crate::ambient::digest::TurnEnding;
use crate::backend::blockcontroller::health::TriggerKind;

use super::{SupervisorDecision, SupervisorMode, SupervisorState, SupervisorStatus, Verdict};

/// What the watcher knows about a turn that just ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnFacts {
    /// What started the turn; `None` when the ledger didn't say, or for the
    /// judgement a mode change asks for.
    pub trigger: Option<TriggerKind>,
    /// Model calls in the turn (`result.num_turns`): 1 means it used no tool.
    pub steps: u32,
    /// How the conversation ends, from the digest.
    pub ending: TurnEnding,
    /// The agent has background tasks still running.
    pub background_running: bool,
    /// A fingerprint of the agent's last final message.
    pub reply: String,
    pub now_ms: u64,
}

/// What happens before any model is asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreJudge {
    /// Nothing to do for this turn.
    Ignore,
    /// Stop for the user without asking the judge.
    Pause(String),
    /// Background work will wake the agent; judge that turn instead.
    Wait,
    Judge,
}

/// Take a turn that ended into `state`, and say whether it is judged. Updates
/// the run counters: a turn the user started resets the run (spec §3.4); a
/// turn the Supervisor started counts toward no-progress.
pub fn on_turn_end(state: &mut SupervisorState, mode: SupervisorMode, facts: &TurnFacts) -> PreJudge {
    if mode == SupervisorMode::Off || facts.trigger == Some(TriggerKind::System) {
        return PreJudge::Ignore;
    }
    if state.mode != mode {
        *state = SupervisorState { history: std::mem::take(&mut state.history), ..SupervisorState::new(mode, facts.now_ms) };
    }
    match facts.trigger {
        Some(TriggerKind::User | TriggerKind::Broadcast) => state.reset_run(),
        Some(TriggerKind::Supervisor) => {
            if facts.steps <= 1 {
                state.no_progress += 1;
            } else {
                state.no_progress = 0;
            }
        }
        _ => {}
    }
    match facts.ending {
        TurnEnding::AskedUser => PreJudge::Pause("It asked you a question.".to_string()),
        // The user's message has no answer yet: nothing of the agent's to judge.
        TurnEnding::UserLast => PreJudge::Ignore,
        _ if facts.background_running => PreJudge::Wait,
        _ => PreJudge::Judge,
    }
}

/// What the Supervisor does with the judge's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Send continuation number `n` of the mode's limit, for the judge's `reason`.
    Continue { n: u32, reason: String },
    /// Stop for the user.
    Pause(String),
    Done(String),
    /// The judge gave no usable verdict: send nothing, notify as usual.
    NoVerdict,
}

/// The action for a verdict, after the limits (spec §3.4). `None` is no verdict.
pub fn decide(state: &SupervisorState, mode: SupervisorMode, verdict: Option<(Verdict, &str)>, facts: &TurnFacts) -> Action {
    let Some((verdict, reason)) = verdict else {
        return Action::NoVerdict;
    };
    match verdict {
        Verdict::Done => Action::Done(reason.to_string()),
        Verdict::NeedsUser => Action::Pause(reason.to_string()),
        Verdict::Blocked => Action::Pause(format!("Blocked: {reason}")),
        Verdict::Continue => match limit_reached(state, mode, facts) {
            Some(why) => Action::Pause(why),
            None => Action::Continue { n: state.continuations + 1, reason: reason.to_string() },
        },
    }
}

/// Why another continuation would pass a limit, or `None`.
pub fn limit_reached(state: &SupervisorState, mode: SupervisorMode, facts: &TurnFacts) -> Option<String> {
    let limits = mode.limits();
    if state.continuations >= limits.continuations {
        return Some(format!("{} continuations in a row; over to you.", state.continuations));
    }
    if let Some(run_ms) = limits.run_ms {
        if state.run_started_at_ms > 0 && facts.now_ms.saturating_sub(state.run_started_at_ms) >= run_ms {
            return Some(format!("Kept going for {} hours; over to you.", run_ms / 3_600_000));
        }
    }
    if state.no_progress >= limits.no_progress {
        return Some(format!("No progress: {} continued turns without using a tool.", state.no_progress));
    }
    let continued = facts.trigger == Some(TriggerKind::Supervisor);
    if continued && !facts.reply.is_empty() && facts.reply == state.last_reply {
        return Some("It gave the same reply twice.".to_string());
    }
    None
}

/// Record `action` in `state` (status, counters, history).
pub fn apply(state: &mut SupervisorState, action: &Action, verdict: Option<Verdict>, facts: &TurnFacts) {
    use super::SupervisorAction as A;
    let (status, history_action, reason) = match action {
        Action::Continue { n, reason } => {
            state.continuations = *n;
            if state.run_started_at_ms == 0 {
                state.run_started_at_ms = facts.now_ms;
            }
            (SupervisorStatus::Continued, A::Continued, reason.clone())
        }
        Action::Pause(why) => (SupervisorStatus::Paused, A::Paused, why.clone()),
        Action::Done(why) => (SupervisorStatus::Done, A::Done, why.clone()),
        Action::NoVerdict => (
            SupervisorStatus::Watching,
            A::NoVerdict,
            "Couldn't judge this turn, so it stopped as usual.".to_string(),
        ),
    };
    state.last_reply = facts.reply.clone();
    state.status = status;
    state.reason = reason.clone();
    state.limit = state.mode.limits().continuations;
    state.record(SupervisorDecision { at_ms: facts.now_ms, action: history_action, verdict, reason });
}

/// Record a stop that needed no judge (`PreJudge::Pause`).
pub fn apply_pause(state: &mut SupervisorState, why: &str, facts: &TurnFacts) {
    apply(state, &Action::Pause(why.to_string()), None, facts);
}

/// Record that background work is running.
pub fn apply_wait(state: &mut SupervisorState, facts: &TurnFacts) {
    let reason = "Waiting for its background work to finish.".to_string();
    state.status = SupervisorStatus::Waiting;
    state.reason = reason.clone();
    state.last_reply = facts.reply.clone();
    state.record(SupervisorDecision { at_ms: facts.now_ms, action: super::SupervisorAction::Waited, verdict: None, reason });
}
