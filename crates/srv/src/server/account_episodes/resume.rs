// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! After a confirmed sign-in, an agent whose last turn failed on the expired login is told, and answers
//! with where it stands (task #2458).
//!
//! Clearing the stored failure ([`super::clear_signed_out`]) lifts the error from the pane and wakes
//! the messages held for it, but a pane with nothing held sat there: the turn that failed never ran
//! again, and the person had to type "u there". Each pane that was cleared of an Auth failure now gets
//! one `[AgentMux]` message that starts a turn: when it signed in, when the turn failed, and to carry on
//! and say in a line or two where things stand. The input that started the failed turn is not sent again
//! because AgentMux does not have it for every kind (a CLI's own background-command completion is never
//! seen here), and the agent's session already holds it: the note is the same for all of them.
//!
//! - **Only a pane whose last turn failed on the login** (the stored failure is `Auth`), and not one
//!   already in a turn: a pane that did not fail gets nothing.
//! - **Paced**: [`BATCH`] panes at a time, [`BATCH_GAP`] apart, since many agents share one account and
//!   each note starts a model turn.
//! - **Once**: a sign-in is often reported twice (the OAuth handler and the window), so a pane told in
//!   the last [`ONCE_MS`] is not told again.
//! - A pane with no controller (a stopped agent) is not started; the note waits for its next turn.
//!   A one-shot (container) pane gets a turn started the way any message to it does.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::task::JoinSet;

/// How many panes are told at once.
pub(super) const BATCH: usize = 3;

/// The wait between groups.
pub(super) const BATCH_GAP: Duration = Duration::from_secs(2);

/// A pane told this recently is not told again for the same sign-in.
const ONCE_MS: i64 = 60_000;

/// The held note's key: a newer one replaces an older one for the pane.
const NOTE_KEY: &str = "signed-in-again";

/// A pane whose last turn failed on the expired login, as it was when the sign-in cleared it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Failed {
    pub block_id: String,
    /// When the failure was recorded (unix ms), if it says.
    pub failed_at_ms: Option<u64>,
}

/// The note for one pane: what happened, and what is asked of the agent.
pub(super) fn note_for(signed_in_ms: i64, failed_at_ms: Option<u64>) -> String {
    use crate::muxbus::delivery_status::notes::hhmm;
    let turn = match failed_at_ms {
        Some(at) => format!("Your turn at {}", hhmm(i64::try_from(at).unwrap_or(0))),
        None => "Your last turn".to_string(),
    };
    format!(
        "Signed in again at {}. {turn} did not run because the login had expired. Carry on from where you were and say in a line or two where things stand.",
        hhmm(signed_in_ms)
    )
}

/// `failed` in groups of [`BATCH`], in the order given.
pub(super) fn batches(failed: Vec<Failed>) -> Vec<Vec<Failed>> {
    failed.chunks(BATCH).map(<[Failed]>::to_vec).collect()
}

/// `failed` without the panes told within [`ONCE_MS`] of `now_ms`, which are recorded as told now.
pub(super) fn not_just_told(told: &mut HashMap<String, i64>, failed: Vec<Failed>, now_ms: i64) -> Vec<Failed> {
    told.retain(|_, at| now_ms - *at < ONCE_MS);
    failed
        .into_iter()
        .filter(|f| {
            if told.contains_key(&f.block_id) {
                return false;
            }
            told.insert(f.block_id.clone(), now_ms);
            true
        })
        .collect()
}

/// What was done for one pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Told {
    /// The note started a turn.
    Turn,
    /// The pane is not running; the note waits for its next turn.
    Held,
    /// The pane is in a turn already; it has the sign-in, so nothing is added.
    Busy,
    /// The turn could not be started; the note waits for the next one.
    Failed,
}

/// Tell one pane. Blocking: starting a turn can wait for a process.
fn tell(block_id: &str, text: &str) -> Told {
    let Some(status) = crate::backend::blockcontroller::get_block_controller_status(block_id) else {
        crate::backend::blockcontroller::system_note_into_turn_or_hold(block_id, NOTE_KEY, text);
        return Told::Held;
    };
    if status.turn_active {
        return Told::Busy;
    }
    let marked = crate::backend::reactive::sanitize::with_system_note_marker(text);
    match crate::backend::reactive::get_global_handler().send_system_turn(block_id, &marked) {
        Ok(_) => Told::Turn,
        Err(e) => {
            tracing::warn!(block_id = %block_id, error = %e, "sign-in: couldn't start the turn that tells an agent it is signed in; the note waits");
            crate::backend::blockcontroller::system_note_into_turn_or_hold(block_id, NOTE_KEY, text);
            Told::Failed
        }
    }
}

async fn run(failed: Vec<Failed>, signed_in_ms: i64) {
    for (i, group) in batches(failed).into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(BATCH_GAP).await;
        }
        let mut set = JoinSet::new();
        for f in group {
            let text = note_for(signed_in_ms, f.failed_at_ms);
            set.spawn_blocking(move || (tell(&f.block_id, &text), f.block_id));
        }
        while let Some(done) = set.join_next().await {
            if let Ok((told, block_id)) = done {
                tracing::info!(block_id = %block_id, ?told, "sign-in: an agent whose turn failed on the login was told");
            }
        }
    }
}

/// Tell each of `failed`, paced and once, in the background. Does nothing outside a runtime.
pub(super) fn start(failed: Vec<Failed>, signed_in_ms: i64) {
    static TOLD: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
    let failed = not_just_told(&mut TOLD.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner()), failed, signed_in_ms);
    if failed.is_empty() {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        tracing::warn!(panes = failed.len(), "sign-in: no runtime to tell the signed-in agents on");
        return;
    };
    tracing::info!(panes = failed.len(), batch = BATCH, gap_ms = BATCH_GAP.as_millis() as u64, "sign-in: telling the agents whose turn failed on the login");
    handle.spawn(run(failed, signed_in_ms));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(ids: &[&str]) -> Vec<Failed> {
        ids.iter().map(|id| Failed { block_id: id.to_string(), failed_at_ms: Some(1_000) }).collect()
    }

    #[test]
    fn the_note_says_when_it_signed_in_and_which_turn_did_not_run_and_asks_for_the_status() {
        let note = note_for(1_700_000_100_000, Some(1_700_000_000_000));
        assert!(note.starts_with("Signed in again at "), "{note}");
        assert!(note.contains("Your turn at ") && note.contains("did not run because the login had expired"), "{note}");
        assert!(note.ends_with("say in a line or two where things stand."), "{note}");
        let unknown = note_for(1_700_000_100_000, None);
        assert!(unknown.contains("Your last turn did not run"), "{unknown}");
    }

    #[test]
    fn panes_are_told_a_few_at_a_time_in_the_order_they_failed_in() {
        let groups = batches(failed(&["a", "b", "c", "d", "e", "f", "g"]));
        let ids: Vec<Vec<&str>> = groups.iter().map(|g| g.iter().map(|f| f.block_id.as_str()).collect()).collect();
        assert_eq!(ids, [vec!["a", "b", "c"], vec!["d", "e", "f"], vec!["g"]]);
        assert!(groups.iter().all(|g| g.len() <= BATCH));
        assert!(batches(Vec::new()).is_empty(), "no pane failed: nothing to tell");
    }

    #[test]
    fn a_pane_told_a_moment_ago_is_not_told_again_but_is_after_a_while() {
        let mut told = HashMap::new();
        assert_eq!(not_just_told(&mut told, failed(&["a", "b"]), 1_000).len(), 2);
        assert!(not_just_told(&mut told, failed(&["a", "b"]), 2_000).is_empty(), "the second report of the same sign-in");
        assert_eq!(not_just_told(&mut told, failed(&["a", "c"]), 3_000), failed(&["c"]));
        assert_eq!(not_just_told(&mut told, failed(&["a"]), 1_000 + ONCE_MS).len(), 1, "a later failure is told");
    }

    #[test]
    fn a_pane_with_no_controller_keeps_the_note_for_its_next_turn_and_is_not_started() {
        let block = format!("resume-test-{}", uuid::Uuid::new_v4());
        assert_eq!(tell(&block, "Signed in again at 09:05."), Told::Held);
        let (message, _held) = crate::backend::reactive::held_notes::attach(&block, "hello");
        assert!(message.contains("Signed in again at 09:05."), "{message}");
    }
}
