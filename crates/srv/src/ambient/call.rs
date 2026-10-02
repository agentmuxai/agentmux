// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The one path every ambient Haiku call takes: admit through the gateway,
//! optionally wait for a concurrency permit, do the caller's own preparation,
//! then run the CLI and validate the reply.
//!
//! ```ignore
//! let slot = call::admit(key, generation, Some(limits::pull_call_semaphore())).await?;
//! // ... read the block, build the prompt (slot.cancellation() is available) ...
//! let reply = slot.run(&target, &prompt, |t| validate::accept_line(t, &limits)).await;
//! ```
//!
//! Before this, six call sites each hand-wrote the same admit / permit / invoke /
//! drop-the-guard / trim-and-empty-check sequence, and only one of them
//! validated what the model said.
//!
//! The slot holds the gateway guard and the permit, so every return path
//! (including an early `?` while preparing) releases both, and a newer request
//! for the same key still cancels this one's CLI.

use tokio::sync::{Semaphore, SemaphorePermit};
use tokio_util::sync::CancellationToken;

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

/// What a call produced. `text` is already validated: empty means the model had
/// nothing usable to say (or the call failed or was superseded). `tokens` is
/// reported whenever the CLI reported usage, including when the text is withheld,
/// because the spend happened either way.
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
    purpose: &'static str,
    entity_id: String,
    cancel: CancellationToken,
    _permit: Option<SemaphorePermit<'static>>,
    _guard: AmbientCallGuard<'static>,
}

/// Admit a call for `key` at `generation`; `None` if it is stale on arrival, or
/// was superseded while queued for a permit. With a `limit`, waits for a permit
/// raced against cancellation, so a request superseded in the queue never spawns
/// a CLI at all.
pub async fn admit(
    key: AmbientCallKey,
    generation: u64,
    limit: Option<&'static Semaphore>,
) -> Option<Slot> {
    let purpose = key.purpose;
    let entity_id = key.entity_id.clone();
    let guard = match gateway().admit(key, generation) {
        Admission::Proceed(guard) => guard,
        Admission::StaleOnArrival => {
            super::outcome::record(purpose, &entity_id, super::outcome::Outcome::Superseded, None);
            return None;
        }
    };
    let cancel = guard.cancellation();
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
                super::outcome::record(purpose, &entity_id, super::outcome::Outcome::Superseded, None);
            }
            Some(permit?)
        }
    };
    Some(Slot { purpose, entity_id, cancel, _permit: permit, _guard: guard })
}

impl Slot {
    /// Fires when a newer request for the same key supersedes this one.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Run the CLI with `prompt` and keep the reply only if `accept` takes it.
    /// Never fails: a CLI error, a cancellation and a rejected reply all come
    /// back as a `Reply` with empty text.
    pub async fn run(
        self,
        target: &CliTarget,
        prompt: &str,
        accept: impl Fn(&str) -> Option<String>,
    ) -> Reply {
        let result = super::cli::invoke_haiku(
            &target.cli_path,
            prompt,
            &target.meta,
            self.cancel.clone(),
        )
        .await;
        match result {
            Err(error) => {
                let outcome = super::outcome::classify_error(&error, self.cancel.is_cancelled());
                tracing::debug!(purpose = self.purpose, entity = %self.entity_id, error = %error, "ambient call failed");
                super::outcome::record(self.purpose, &self.entity_id, outcome, None);
                Reply { text: String::new(), tokens: None, error: Some(error) }
            }
            Ok((raw, tokens)) => {
                let text = accept(&raw).unwrap_or_default();
                let outcome = super::outcome::classify_reply(&raw, !text.is_empty());
                super::outcome::record(self.purpose, &self.entity_id, outcome, Some(&raw));
                Reply { text, tokens, error: None }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(entity: &str, purpose: &'static str) -> AmbientCallKey {
        AmbientCallKey::new(entity, purpose)
    }

    fn leaked_semaphore(permits: usize) -> &'static Semaphore {
        Box::leak(Box::new(Semaphore::new(permits)))
    }

    #[tokio::test]
    async fn a_stale_request_is_not_admitted() {
        let _newer = admit(key("call-stale", "t_stale"), 5, None).await.expect("first is admitted");
        assert!(admit(key("call-stale", "t_stale"), 5, None).await.is_none(), "same generation");
        assert!(admit(key("call-stale", "t_stale"), 4, None).await.is_none(), "older generation");
    }

    #[tokio::test]
    async fn a_newer_request_cancels_the_older_ones_cli() {
        let first = admit(key("call-newer", "t_newer"), 1, None).await.unwrap();
        let cancelled = first.cancellation();
        let _second = admit(key("call-newer", "t_newer"), 2, None).await.unwrap();
        assert!(cancelled.is_cancelled());
    }

    #[tokio::test]
    async fn different_entities_do_not_cancel_each_other() {
        let first = admit(key("call-a", "t_sib"), 1, None).await.unwrap();
        let _second = admit(key("call-b", "t_sib"), 1, None).await.unwrap();
        assert!(!first.cancellation().is_cancelled());
    }

    #[tokio::test]
    async fn the_permit_is_held_until_the_slot_drops() {
        let sem = leaked_semaphore(1);
        let slot = admit(key("call-permit", "t_permit"), 1, Some(sem)).await.unwrap();
        assert_eq!(sem.available_permits(), 0);
        drop(slot);
        assert_eq!(sem.available_permits(), 1);
    }

    #[tokio::test]
    async fn a_request_superseded_while_queued_never_gets_a_permit() {
        let sem = leaked_semaphore(1);
        let holder = admit(key("call-holder", "t_queue"), 1, Some(sem)).await.unwrap();

        // Queues behind the holder (a different entity, so the holder is not cancelled).
        let queued = tokio::spawn(admit(key("call-queued", "t_queue"), 1, Some(sem)));
        tokio::task::yield_now().await;
        // A newer request for the queued key supersedes it while it waits.
        let newer = admit(key("call-queued", "t_queue"), 2, None).await.unwrap();

        assert!(queued.await.unwrap().is_none(), "superseded in the queue");
        drop(holder);
        drop(newer);
        assert_eq!(sem.available_permits(), 1, "no permit leaked");
    }

    #[tokio::test]
    async fn dropping_the_slot_lets_the_same_generation_be_admitted_again_only_if_newer() {
        let slot = admit(key("call-redo", "t_redo"), 3, None).await.unwrap();
        drop(slot);
        // The gateway clears the in-flight entry on drop, so the next request admits.
        assert!(admit(key("call-redo", "t_redo"), 3, None).await.is_some());
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
