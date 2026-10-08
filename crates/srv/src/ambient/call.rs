// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one path every ambient Haiku call takes: admit through the gateway,
//! optionally wait for a concurrency permit, do the caller's own preparation,
//! then run the CLI and judge the reply.
//!
//! ```ignore
//! let slot = call::admit(&purpose::ACTIVITY_SUMMARY, block_id, generation).await?;
//! // ... read the block, build the prompt (slot.cancellation() is available) ...
//! let reply = slot.run(&target, &prompt, |raw| reply::judge_line(raw, |t| validate::accept_line(t, &limits))).await;
//! ```
//!
//! The slot holds the gateway guard and the permit, so every return path
//! (including an early `?` while preparing) releases both, and a newer request
//! for the same key still cancels this one's CLI. Every admitted call ends in
//! exactly one recorded outcome, taken from the judge's [`Verdict`].

use tokio::sync::{Semaphore, SemaphorePermit};
use tokio_util::sync::CancellationToken;

use super::limits::Class;
use super::outcome::{Outcome, Timing};
use super::purpose::Purpose;
use super::reply::Verdict;
use super::{gateway, Admission, AmbientCallGuard, AmbientCallKey};
use crate::agents::TokenCounts;
use crate::backend::obj::{self, MetaMapType};

/// Which CLI to run, and the block meta carrying its auth env (`cmd:env`).
pub struct CliTarget {
    pub cli_path: String,
    pub meta: MetaMapType,
}

impl CliTarget {
    /// From a block's meta; `None` when it names no CLI (`cmd`).
    pub fn from_meta(meta: &MetaMapType) -> Option<Self> {
        let cli_path = obj::meta_get_string(meta, "cmd", "");
        if cli_path.is_empty() {
            return None;
        }
        Some(Self { cli_path, meta: meta.clone() })
    }
}

/// What a call produced. `text` is already judged: empty means there is nothing
/// to use (the model skipped, the reply was refused, or the call failed or was
/// superseded). `tokens` is reported whenever the CLI reported usage, including
/// when the text is withheld, because the spend happened either way.
#[derive(Debug, Default)]
pub struct Reply {
    pub text: String,
    pub tokens: Option<TokenCounts>,
    /// Why the CLI call failed, if it did. Already logged at debug level.
    pub error: Option<String>,
}

impl Reply {
    /// `Some((text, tokens))` only when there is text to use.
    pub fn into_option(self) -> Option<(String, Option<TokenCounts>)> {
        if self.text.is_empty() {
            None
        } else {
            Some((self.text, self.tokens))
        }
    }
}

/// An admitted call. Dropping it (any path) releases the gateway entry and the
/// permit.
pub struct Slot {
    purpose: &'static Purpose,
    entity_id: String,
    cancel: CancellationToken,
    _permit: Option<SemaphorePermit<'static>>,
    _guard: AmbientCallGuard<'static>,
    /// Set once this call's outcome is recorded, so `Drop` records `NotRun` only
    /// for a call given up without one: every admitted call ends in exactly one
    /// outcome.
    recorded: bool,
    /// Time spent waiting for a concurrency permit after admission.
    queued_ms: u64,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if !self.recorded {
            super::outcome::record(self.purpose.tag, &self.entity_id, Outcome::NotRun, None);
        }
    }
}

/// Admit a call for `purpose` on `entity_id` at `generation`, in the purpose's
/// own class (`limits`); `None` if it is stale on arrival, or was superseded while
/// queued. It waits for a permit raced against cancellation, so a request
/// superseded in the queue never spawns a CLI at all.
pub async fn admit(purpose: &'static Purpose, entity_id: impl Into<String>, generation: u64) -> Option<Slot> {
    admit_with_limit(purpose, entity_id, generation, Some(purpose.class.semaphore())).await
}

/// [`admit`] in another class than the purpose's own: an interactive purpose run
/// as background work, such as the backlog pass naming historical subagents.
pub async fn admit_as(
    purpose: &'static Purpose,
    entity_id: impl Into<String>,
    generation: u64,
    class: Class,
) -> Option<Slot> {
    admit_with_limit(purpose, entity_id, generation, Some(class.semaphore())).await
}

async fn admit_with_limit(
    purpose: &'static Purpose,
    entity_id: impl Into<String>,
    generation: u64,
    limit: Option<&'static Semaphore>,
) -> Option<Slot> {
    let entity_id = entity_id.into();
    let guard = match gateway().admit(AmbientCallKey::new(entity_id.clone(), purpose.tag), generation) {
        Admission::Proceed(guard) => guard,
        Admission::StaleOnArrival => {
            super::outcome::record(purpose.tag, &entity_id, Outcome::Superseded, None);
            return None;
        }
    };
    let cancel = guard.cancellation();
    let admitted_at = std::time::Instant::now();
    let permit = match limit {
        None => None,
        Some(sem) => {
            let permit = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                permit = sem.acquire() => permit.ok(),
            };
            if permit.is_none() {
                // Superseded while queued for a permit: never spawned.
                super::outcome::record(purpose.tag, &entity_id, Outcome::Superseded, None);
            }
            Some(permit?)
        }
    };
    let queued_ms = admitted_at.elapsed().as_millis() as u64;
    Some(Slot { purpose, entity_id, cancel, _permit: permit, _guard: guard, recorded: false, queued_ms })
}

impl Slot {
    /// Fires when a newer request for the same key supersedes this one.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Give the call up without running it, recording why (for example
    /// `EmptyDigest`), instead of the `NotRun` a plain drop records.
    pub fn abandon(mut self, outcome: Outcome) {
        super::outcome::record(self.purpose.tag, &self.entity_id, outcome, None);
        self.recorded = true;
    }

    /// Run the CLI with `prompt`, within the purpose's time limit, and let `judge`
    /// decide what the reply is worth (`reply::judge_line` for a one-line reply in
    /// the `ANSWER:`/`SKIP` format). Never fails: a CLI error, a cancellation, a
    /// skip and a refused reply all come back as a `Reply` with empty text.
    pub async fn run(mut self, target: &CliTarget, prompt: &str, judge: impl Fn(&str) -> Verdict) -> Reply {
        let started = std::time::Instant::now();
        let result = super::cli::invoke_haiku(
            &target.cli_path,
            prompt,
            &target.meta,
            self.cancel.clone(),
            self.purpose.timeout,
        )
        .await;
        let timing = Timing { queued_ms: self.queued_ms, run_ms: started.elapsed().as_millis() as u64 };
        self.recorded = true;
        match result {
            Err(error) => {
                let outcome = super::outcome::classify_error(&error, self.cancel.is_cancelled());
                tracing::debug!(purpose = self.purpose.tag, entity = %self.entity_id, error = %error, "ambient call failed");
                super::outcome::record_timed(self.purpose.tag, &self.entity_id, outcome, None, Some(timing));
                Reply { text: String::new(), tokens: None, error: Some(error) }
            }
            Ok((raw, tokens)) => {
                let (text, outcome) = match judge(&raw) {
                    Verdict::Use(text) => (text, Outcome::Accepted),
                    Verdict::Skip => (String::new(), Outcome::Skipped),
                    Verdict::Reject(reason) => (String::new(), Outcome::Rejected(reason)),
                };
                let logged = self.purpose.logs_reply.then_some(raw.as_str());
                super::outcome::record_timed(self.purpose.tag, &self.entity_id, outcome, logged, Some(timing));
                Reply { text, tokens, error: None }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Test purposes, each its own tag so parallel tests never share a gateway
    /// key or an outcome counter.
    macro_rules! test_purpose {
        ($name:ident, $tag:literal) => {
            static $name: Purpose =
                Purpose { tag: $tag, timeout: Duration::from_secs(1), class: Class::Interactive, logs_reply: true };
        };
    }
    test_purpose!(OUTCOMES, "test_purpose_slot_outcomes");
    test_purpose!(STALE, "t_stale");
    test_purpose!(NEWER, "t_newer");
    test_purpose!(SIBLINGS, "t_sib");
    test_purpose!(PERMIT, "t_permit");
    test_purpose!(QUEUE, "t_queue");
    test_purpose!(REDO, "t_redo");

    fn leaked_semaphore(permits: usize) -> &'static Semaphore {
        Box::leak(Box::new(Semaphore::new(permits)))
    }

    /// An admitted call given up before running (no block, no CLI path) still ends
    /// in one outcome, and an explicit `abandon` records its own outcome instead of
    /// `not_run`, never both.
    #[tokio::test]
    async fn every_admitted_call_ends_in_exactly_one_outcome() {
        let count = |label: &str| {
            crate::ambient::outcome::snapshot()
                .get(OUTCOMES.tag)
                .and_then(|by| by.get(label).copied())
                .unwrap_or(0)
        };
        let dropped = admit_with_limit(&OUTCOMES, "e1", 1, None).await.unwrap();
        drop(dropped);
        assert_eq!(count("not_run"), 1);

        let abandoned = admit_with_limit(&OUTCOMES, "e2", 1, None).await.unwrap();
        abandoned.abandon(Outcome::EmptyDigest);
        assert_eq!(count("empty_digest"), 1);
        assert_eq!(count("not_run"), 1, "abandon records once, not twice");
    }

    #[tokio::test]
    async fn a_stale_request_is_not_admitted() {
        let _newer = admit_with_limit(&STALE, "call-stale", 5, None).await.expect("first is admitted");
        assert!(admit_with_limit(&STALE, "call-stale", 5, None).await.is_none(), "same generation");
        assert!(admit_with_limit(&STALE, "call-stale", 4, None).await.is_none(), "older generation");
    }

    #[tokio::test]
    async fn a_newer_request_cancels_the_older_ones_cli() {
        let first = admit_with_limit(&NEWER, "call-newer", 1, None).await.unwrap();
        let cancelled = first.cancellation();
        let _second = admit_with_limit(&NEWER, "call-newer", 2, None).await.unwrap();
        assert!(cancelled.is_cancelled());
    }

    #[tokio::test]
    async fn different_entities_do_not_cancel_each_other() {
        let first = admit_with_limit(&SIBLINGS, "call-a", 1, None).await.unwrap();
        let _second = admit_with_limit(&SIBLINGS, "call-b", 1, None).await.unwrap();
        assert!(!first.cancellation().is_cancelled());
    }

    #[tokio::test]
    async fn the_permit_is_held_until_the_slot_drops() {
        let sem = leaked_semaphore(1);
        let slot = admit_with_limit(&PERMIT, "call-permit", 1, Some(sem)).await.unwrap();
        assert_eq!(sem.available_permits(), 0);
        drop(slot);
        assert_eq!(sem.available_permits(), 1);
    }

    #[tokio::test]
    async fn a_request_superseded_while_queued_never_gets_a_permit() {
        let sem = leaked_semaphore(1);
        let holder = admit_with_limit(&QUEUE, "call-holder", 1, Some(sem)).await.unwrap();

        // Queues behind the holder (a different entity, so the holder is not cancelled).
        let queued = tokio::spawn(admit_with_limit(&QUEUE, "call-queued", 1, Some(sem)));
        tokio::task::yield_now().await;
        // A newer request for the queued key supersedes it while it waits.
        let newer = admit_with_limit(&QUEUE, "call-queued", 2, None).await.unwrap();

        assert!(queued.await.unwrap().is_none(), "superseded in the queue");
        drop(holder);
        drop(newer);
        assert_eq!(sem.available_permits(), 1, "no permit leaked");
    }

    #[tokio::test]
    async fn dropping_the_slot_lets_the_same_generation_be_admitted_again_only_if_newer() {
        let slot = admit_with_limit(&REDO, "call-redo", 3, None).await.unwrap();
        drop(slot);
        // The gateway clears the in-flight entry on drop, so the next request admits.
        assert!(admit_with_limit(&REDO, "call-redo", 3, None).await.is_some());
    }

    #[test]
    fn a_reply_with_no_text_has_no_option() {
        assert!(Reply::default().into_option().is_none());
        let r = Reply { text: "x".into(), tokens: None, error: None };
        assert_eq!(r.into_option().map(|(t, _)| t), Some("x".to_string()));
    }

    #[test]
    fn a_target_needs_a_cmd() {
        let mut meta = MetaMapType::new();
        assert!(CliTarget::from_meta(&meta).is_none());
        meta.insert("cmd".into(), serde_json::json!("/usr/bin/claude"));
        assert_eq!(CliTarget::from_meta(&meta).unwrap().cli_path, "/usr/bin/claude");
    }
}
