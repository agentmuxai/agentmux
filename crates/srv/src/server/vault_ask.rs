// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Asking the person to allow an agent's use of a Vault entry, and waiting for
//! how it ended (SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §5):
//! the one way every use (a fill, a command, a capture, a sign-in, a rotation)
//! asks, so each gets srv's deadline, the same attention and the same record of
//! who decided.

use std::time::Duration;

use crate::backend::vault::uses::{self, Ended, UseAnswer, UseRequest};

use super::browser_vault_fill::Waiting;
use super::AppState;

/// How long past its own wait a caller waits, so srv's decision on silence
/// always reaches it first.
const GRACE: Duration = Duration::from_secs(5);

/// The Activity action of a use that nobody answered and that went ahead.
const ASKED: &str = "asked to use it";

/// Asks the person to allow `request` and waits at most `wait`: srv decides on
/// silence then. `attention_block` is the pane whose tone and badges say it
/// waits. A use that went ahead with no answer is recorded as such at once;
/// every other outcome is recorded by the caller, with what it did.
pub(super) async fn ask(state: &AppState, attention_block: &str, request: UseRequest, wait: Duration, headline: &str) -> (Ended, Option<UseAnswer>) {
    let (id, entry, label, agent) = (request.id.clone(), request.entry_id.clone(), request.entry_label.clone(), request.agent.clone());
    let target = target_of(&request);
    let answer = uses::uses().ask(request, wait);
    let _waiting = Waiting::new(id, state.clone());
    super::vault_handlers::publish(state);
    let _asking = crate::backend::user_attention::Asking::start(&state.broker, &state.mstore, attention_block, "vault", headline);
    let answer = match tokio::time::timeout(wait + GRACE, answer).await {
        Ok(Ok(a)) => Some(a),
        // The request was taken down without an answer: as a no.
        Ok(Err(_)) => Some(UseAnswer::of(uses::Decision::Deny)),
        Err(_) => None,
    };
    let ended = Ended::of(answer.as_ref());
    if ended == Ended::WentAhead {
        crate::backend::vault::record(&state.identity_store, &entry, &label, &agent, ASKED, &target, uses::WENT_AHEAD_OUTCOME);
    }
    (ended, answer)
}

/// `out`, a successful answer to the agent, saying it went ahead because nobody
/// answered, when it did (spec §5.1).
pub(super) fn with_approval(mut out: serde_json::Value, went_ahead: bool) -> serde_json::Value {
    if went_ahead {
        out["approval"] = serde_json::json!(uses::WENT_AHEAD);
    }
    out
}

/// What a command's agent is told about the entries it was given because
/// nobody answered, if any.
pub(super) fn approval(labels: &[String]) -> Option<String> {
    (!labels.is_empty()).then(|| format!("{} ({})", uses::WENT_AHEAD, labels.join(", ")))
}

/// Where the use was for, as the Activity line names it.
fn target_of(r: &UseRequest) -> String {
    if !r.command.is_empty() {
        r.command.chars().take(160).collect()
    } else if r.field.is_empty() {
        r.origin.clone()
    } else {
        format!("{} ({})", r.origin, r.field)
    }
}

/// The asking agent's own pane, for its request: the card is answered there.
/// Empty when the agent's identity doesn't name one.
pub(super) fn agent_pane(state: &AppState, caller: Option<&crate::server::caller::Caller>, auth: &agentmux_common::api_types::UiAutomationAuth) -> String {
    super::ui_handlers::verified_block_id(state, caller, auth).unwrap_or_default()
}
