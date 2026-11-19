// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The person's words to an agent that is out of quota.
//!
//! A Swarm broadcast to an agent marked `limited` used to be refused ("not sent") and nothing
//! looked at whether the limit was still real; a quota refilled before the provider's reset left the
//! agent limited for days. Now the words are kept, a check turn starts on the agent at once
//! (`limit_probe::check_now`, no wait for the 15-minute slot a message between agents has), and the
//! broadcast's answer waits [`CHECK_WAIT_MS`] for the result: delivered when the check cleared the
//! limit, otherwise "held", with the reason. Kept words are sent as soon as the agent can work,
//! whoever cleared it (the check, a clean turn of another agent on its account, the reset), and
//! for at most [`KEEP_MS`]. The words never ride the check turn: a check that fails leaves the agent
//! limited and the words kept.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;

use crate::backend::message_origin::MessageOrigin;
use crate::backend::agent_state::{AgentState, Unavailable};
use crate::server::agent_handlers::AgentTurnDeps;

/// An answer that starts with this is "kept, not sent yet", not a failure.
pub(crate) const HELD_PREFIX: &str = "held: ";
/// How long a broadcast's answer waits for the check.
const CHECK_WAIT_MS: u64 = 30_000;
/// How often the watcher looks while the check is fresh, then after [`FAST_FOR_MS`].
const POLL_FAST_MS: u64 = 1_000;
const POLL_SLOW_MS: u64 = 5_000;
const FAST_FOR_MS: u64 = 60_000;
/// How long words are kept for an agent that stays out of quota (the held-message hold's 24 hours).
const KEEP_MS: u64 = 24 * 60 * 60 * 1000;
/// The most messages kept for one agent.
const MAX_KEPT: usize = 20;

struct Words {
    text: String,
    from: MessageOrigin,
    kept_at_ms: u64,
    /// Where the answer to the sender goes: delivered, or why it wasn't. Dropped when the sender stopped waiting.
    answer: oneshot::Sender<Result<(), String>>,
}

/// The words kept per pane, oldest first.
static KEPT: LazyLock<Mutex<HashMap<String, Vec<Words>>>> = LazyLock::new(Default::default);

/// The reason in a broadcast answer that means "kept, not sent yet" ([`HELD_PREFIX`]); `None` for a failure.
pub(crate) fn held_note(error: &str) -> Option<&str> {
    error.strip_prefix(HELD_PREFIX)
}

/// Whether a message to an agent in this state is kept and checked, rather than refused: a plan's
/// limit (not a route out of credits, and not a sign-in: neither ends by itself being asked).
pub(crate) fn is_kept_for_check(unavailable: &Unavailable) -> bool {
    unavailable.state == AgentState::Limited && unavailable.title != crate::agents::failure::ROUTE_CREDITS_TITLE
}

/// Keeps `words` for `block_id`; `true` when it is the first (the caller starts the watcher). `Err` when
/// the agent already has [`MAX_KEPT`].
fn keep(block_id: &str, words: Words) -> Result<bool, String> {
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let list = kept.entry(block_id.to_string()).or_default();
    if list.len() >= MAX_KEPT {
        return Err(format!("{}too many messages are already kept for this agent: not sent", crate::backend::reactive::UNAVAILABLE_PREFIX));
    }
    list.push(words);
    Ok(list.len() == 1)
}

/// Takes everything kept for `block_id`.
fn take_all(block_id: &str) -> Vec<Words> {
    KEPT.lock().unwrap_or_else(|e| e.into_inner()).remove(block_id).unwrap_or_default()
}

/// Drops what has been kept [`KEEP_MS`], answering each sender (if still waiting) with that.
fn expire(block_id: &str, now_ms: u64) {
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(list) = kept.get_mut(block_id) else { return };
    let (old, fresh): (Vec<Words>, Vec<Words>) = std::mem::take(list).into_iter().partition(|w| now_ms.saturating_sub(w.kept_at_ms) >= KEEP_MS);
    *list = fresh;
    for w in old {
        let _ = w.answer.send(Err(format!("{}the agent stayed out of quota for 24 hours: not sent", crate::backend::reactive::UNAVAILABLE_PREFIX)));
    }
}

/// How many are kept for `block_id`.
pub(crate) fn kept_count(block_id: &str) -> usize {
    KEPT.lock().unwrap_or_else(|e| e.into_inner()).get(block_id).map_or(0, Vec::len)
}

/// The words of a broadcast to `block_id`, an agent out of quota: kept, the check started, and the
/// answer after [`CHECK_WAIT_MS`] at most. `Ok` when delivered; an `Err` that starts with
/// [`HELD_PREFIX`] when kept and not sent yet.
pub(crate) async fn hold_and_check(
    deps: &AgentTurnDeps,
    block_id: String,
    text: String,
    from: MessageOrigin,
    reason: &str,
) -> Result<(), String> {
    hold_and_check_within(deps, block_id, text, from, reason, Duration::from_millis(CHECK_WAIT_MS)).await
}

/// [`hold_and_check`], with how long the answer waits for the check.
async fn hold_and_check_within(
    deps: &AgentTurnDeps,
    block_id: String,
    text: String,
    from: MessageOrigin,
    reason: &str,
    wait: Duration,
) -> Result<(), String> {
    let (answer, answered) = oneshot::channel();
    let now_ms = agentmux_common::time::now_ms_u64();
    let first = keep(&block_id, Words { text, from, kept_at_ms: now_ms, answer })?;
    if first {
        spawn_watch(deps.clone(), block_id.clone());
    }
    match super::limit_probe::check_now(&block_id, now_ms) {
        super::limit_probe::Check::Started => tracing::info!(block_id = %block_id, "person hold: started a check turn for the person's message"),
        super::limit_probe::Check::Running => {}
        super::limit_probe::Check::NotStarted(why) => tracing::info!(block_id = %block_id, why = %why, "person hold: no check turn started; the message waits"),
    }
    match tokio::time::timeout(wait, answered).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(format!("{}the message was dropped before it was sent", crate::backend::reactive::UNAVAILABLE_PREFIX)),
        Err(_) => Err(format!(
            "{HELD_PREFIX}{reason}; the check has not cleared it yet. Kept, and sent when it can work (up to 24 hours)"
        )),
    }
}

/// Watches `block_id` until it can work, then sends what is kept, oldest first.
fn spawn_watch(deps: AgentTurnDeps, block_id: String) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
    runtime.spawn(async move {
        let (mstore, id) = (deps.mstore.clone(), block_id.clone());
        let holding = move || {
            let (mstore, id) = (mstore.clone(), id.clone());
            async move { tokio::task::spawn_blocking(move || super::jekt_hold::holding_unavailable(&mstore, &id).is_some()).await.unwrap_or(true) }
        };
        let deliver = {
            let (deps, id) = (deps.clone(), block_id.clone());
            move |text: String, from: MessageOrigin| {
                let (deps, id) = (deps.clone(), id.clone());
                async move { super::app_api::fleet::deliver_available(&deps, id, text, from).await }
            }
        };
        watch(&block_id, holding, deliver, Duration::from_millis(POLL_FAST_MS), Duration::from_millis(POLL_SLOW_MS), Duration::from_millis(FAST_FOR_MS)).await;
    });
}

/// The watcher: looks every `fast` for `fast_for`, then every `slow`, and when `holding` says the agent
/// can work, sends what is kept through `deliver` and answers each sender still waiting. Ends when
/// nothing is kept for `block_id`.
async fn watch<H, HF, D, DF>(block_id: &str, holding: H, deliver: D, fast: Duration, slow: Duration, fast_for: Duration)
where
    H: Fn() -> HF,
    HF: Future<Output = bool>,
    D: Fn(String, MessageOrigin) -> DF,
    DF: Future<Output = Result<(), String>>,
{
    let started = std::time::Instant::now();
    loop {
        tokio::time::sleep(if started.elapsed() < fast_for { fast } else { slow }).await;
        if kept_count(block_id) == 0 {
            return;
        }
        expire(block_id, agentmux_common::time::now_ms_u64());
        if holding().await {
            continue;
        }
        for words in take_all(block_id) {
            let result = deliver(words.text, words.from).await;
            if let Err(e) = &result {
                tracing::info!(block_id = %block_id, error = %e, "person hold: the kept message was not delivered");
            }
            let _ = words.answer.send(result);
        }
        if kept_count(block_id) == 0 {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    fn words(text: &str, kept_at_ms: u64) -> (Words, oneshot::Receiver<Result<(), String>>) {
        let (answer, rx) = oneshot::channel();
        (Words { text: text.into(), from: MessageOrigin::broadcast(), kept_at_ms, answer }, rx)
    }

    fn limited(title: &str, state: AgentState) -> Unavailable {
        Unavailable { state, code: crate::agents::failure::FailureClass::UsageLimit, title: title.into(), resets_at_ms: Some(5), since_ms: 0, account_id: None }
    }

    #[test]
    fn only_a_plans_limit_is_kept_and_checked() {
        assert!(is_kept_for_check(&limited("Usage limit reached", AgentState::Limited)));
        assert!(!is_kept_for_check(&limited(crate::agents::failure::ROUTE_CREDITS_TITLE, AgentState::Limited)), "the route's own episode ends that");
        assert!(!is_kept_for_check(&limited("Needs sign-in", AgentState::SignedOut)));
    }

    #[test]
    fn at_most_the_cap_is_kept_per_agent_and_only_the_first_starts_a_watcher() {
        let id = "person-hold-cap";
        let mut firsts = 0;
        for n in 0..MAX_KEPT {
            if keep(id, words(&format!("m{n}"), 1).0).unwrap() {
                firsts += 1;
            }
        }
        assert_eq!((firsts, kept_count(id)), (1, MAX_KEPT));
        let refused = keep(id, words("one more", 1).0).unwrap_err();
        assert!(refused.starts_with(crate::backend::reactive::UNAVAILABLE_PREFIX), "{refused}");
        assert_eq!(take_all(id).len(), MAX_KEPT);
    }

    #[test]
    fn words_kept_for_a_day_are_dropped_with_that_answer() {
        let id = "person-hold-expiry";
        let (old, mut old_rx) = words("old", 1_000);
        let (fresh, _fresh_rx) = words("fresh", KEEP_MS);
        keep(id, old).unwrap();
        keep(id, fresh).unwrap();
        expire(id, 1_000 + KEEP_MS);
        assert_eq!(take_all(id).iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), vec!["fresh"]);
        let answer = old_rx.try_recv().expect("answered").unwrap_err();
        assert!(answer.contains("24 hours"), "{answer}");
    }

    #[test]
    fn a_kept_answer_is_told_from_a_failure_by_its_prefix() {
        assert_eq!(held_note("held: Out of quota; kept"), Some("Out of quota; kept"));
        assert_eq!(held_note("agent unavailable: Out of quota: not sent"), None);
        assert_eq!(held_note("no registered agent for this block"), None);
    }

    /// The whole of it on a real store: a person's words to a limited agent are kept and answered as kept
    /// (the check cannot start: nothing is registered on the pane), and go the moment the limit is gone,
    /// whoever cleared it.
    #[tokio::test]
    async fn a_broadcast_to_a_limited_agent_is_kept_and_leaves_once_the_limit_clears() {
        let state = crate::server::tests::test_state();
        let deps = AgentTurnDeps::from_state(&state);
        let block_id = "person-hold-e2e-pane".to_string();
        let mut block = crate::backend::obj::Block { oid: block_id.clone(), ..Default::default() };
        let now = agentmux_common::time::now_ms_u64();
        block.meta.insert(
            crate::backend::blockcontroller::core::META_LAST_FAILURE.to_string(),
            serde_json::json!({"code": "usage_limit", "title": "Usage limit reached", "detail": "", "retryable": false, "resetsAtMs": now + 86_400_000, "atMs": now - 1_000}),
        );
        state.mstore.insert(&mut block).unwrap();
        // A pane with a running controller, as a limited agent has.
        crate::backend::agent_state::test_support::register_stub(
            &block_id,
            crate::backend::blockcontroller::BLOCK_CONTROLLER_PERSISTENT,
            crate::backend::blockcontroller::STATUS_RUNNING,
            false,
        );
        let unavailable = crate::server::jekt_hold::holding_unavailable(&state.mstore, &block_id).expect("limited");
        assert!(is_kept_for_check(&unavailable));

        let answer = hold_and_check_within(&deps, block_id.clone(), "quota is back".into(), MessageOrigin::broadcast(), "Out of quota", Duration::from_millis(50)).await;
        let note = answer.as_ref().err().and_then(|e| held_note(e)).expect("kept, not failed");
        assert!(note.contains("Out of quota") && note.contains("sent when it can work"), "{note}");
        assert_eq!(kept_count(&block_id), 1, "the words are kept, not lost");

        // Another agent on the account worked: the stored limit is cleared.
        let mut stored = state.mstore.get::<crate::backend::obj::Block>(&block_id).unwrap().unwrap();
        stored.meta.remove(crate::backend::blockcontroller::core::META_LAST_FAILURE);
        state.mstore.update(&mut stored).unwrap();
        assert!(crate::server::jekt_hold::holding_unavailable(&state.mstore, &block_id).is_none(), "the limit is gone from the store");
        let gone = async {
            while kept_count(&block_id) > 0 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(5), gone).await.expect("the watcher sends what was kept once the agent can work");
        crate::backend::blockcontroller::delete_controller(&block_id);
    }

    #[tokio::test]
    async fn kept_words_go_in_order_once_the_agent_can_work_and_not_before() {
        let id = "person-hold-watch";
        let (a, a_rx) = words("first", agentmux_common::time::now_ms_u64());
        let (b, b_rx) = words("second", agentmux_common::time::now_ms_u64());
        keep(id, a).unwrap();
        keep(id, b).unwrap();

        let limited = Arc::new(AtomicBool::new(true));
        let sent = Arc::new(Mutex::new(Vec::<String>::new()));
        let looks = Arc::new(AtomicUsize::new(0));
        let (l, s, n) = (limited.clone(), sent.clone(), looks.clone());
        let watcher = tokio::spawn({
            let (l2, s2) = (l.clone(), s.clone());
            async move {
                watch(
                    id,
                    move || {
                        n.fetch_add(1, Ordering::SeqCst);
                        let still = l2.load(Ordering::SeqCst);
                        async move { still }
                    },
                    move |text, _from| {
                        s2.lock().unwrap().push(text);
                        async { Ok(()) }
                    },
                    Duration::from_millis(2),
                    Duration::from_millis(2),
                    Duration::from_millis(50),
                )
                .await
            }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while looks.load(Ordering::SeqCst) < 3 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("it looks again and again while the agent is limited");
        assert!(sent.lock().unwrap().is_empty(), "nothing is sent while it is limited");
        assert_eq!(kept_count(id), 2, "and nothing is lost");

        limited.store(false, Ordering::SeqCst);
        watcher.await.unwrap();
        assert_eq!(*sent.lock().unwrap(), vec!["first".to_string(), "second".to_string()]);
        assert_eq!((a_rx.await.unwrap(), b_rx.await.unwrap()), (Ok(()), Ok(())), "each sender still waiting is answered");
        assert_eq!(kept_count(id), 0);
    }

    #[tokio::test]
    async fn a_failed_delivery_is_the_senders_answer_and_does_not_stop_the_rest() {
        let id = "person-hold-fail";
        let now = agentmux_common::time::now_ms_u64();
        let (a, a_rx) = words("bad", now);
        let (b, b_rx) = words("good", now);
        keep(id, a).unwrap();
        keep(id, b).unwrap();
        watch(
            id,
            || async { false },
            |text, _| async move { if text == "bad" { Err("no controller".to_string()) } else { Ok(()) } },
            Duration::from_millis(1),
            Duration::from_millis(1),
            Duration::from_millis(10),
        )
        .await;
        assert_eq!((a_rx.await.unwrap(), b_rx.await.unwrap()), (Err("no controller".to_string()), Ok(())));
    }
}
