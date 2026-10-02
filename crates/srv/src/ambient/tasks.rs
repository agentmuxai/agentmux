// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The background and one-shot ambient Haiku callers: pushed and per-definition
//! activity summaries, subagent and workflow-batch names, and narration. Each is
//! best-effort — every failure path returns `None` and the caller carries on
//! without. They all go through `call::admit` / `Slot::run`, so admission,
//! cancellation, concurrency caps and reply validation are the same for each.
//!
//! The two pull RPCs (`session:activity_summary`, `session:next_prompt_suggestion`)
//! use the same path from `server::app_api::session`.

use std::sync::Arc;

use super::call::{self, CliTarget, Reply};
use super::{digest, limits, prompt, purpose, validate, AmbientCallKey};
use crate::agents::TokenCounts;
use crate::backend::obj::{Block, MetaMapType};
use crate::backend::storage::store::Store;

/// What a background call produced. `None` from a task means it did not run to
/// completion (superseded, capped out, nothing to send, CLI failed), so a caller
/// that tracks progress should try again later. `Some` means the CLI ran: `text`
/// is `None` when the model had nothing usable to say (the reply failed
/// validation, or was empty), which is a finished attempt, not a failure, and
/// `tokens` is the spend either way.
#[derive(Debug)]
pub(crate) struct Generated {
    pub text: Option<String>,
    pub tokens: Option<TokenCounts>,
}

fn finish(reply: Reply) -> Option<Generated> {
    if reply.error.is_some() {
        return None;
    }
    let text = (!reply.text.is_empty()).then_some(reply.text);
    Some(Generated { text, tokens: reply.tokens })
}

/// Read-only CLI path lookup for `provider_id` — checks the versioned
/// local-install dir, then falls back to system PATH. Deliberately never
/// installs anything (unlike the `resolvecli` RPC handler / `agent_open.rs`'s
/// launch-time resolution, which both trigger an npm install as a fallback):
/// this is a best-effort BACKGROUND call, not a user-initiated launch —
/// silently installing a CLI as a side effect of a picker preview summary
/// would be a surprising, unwanted cost. Returns `None` (not an error) for
/// an unknown provider or a CLI that isn't already available either way;
/// the caller treats that the same as any other unresolvable case.
pub async fn resolve_provider_cli_path_readonly(provider_id: &str) -> Option<String> {
    let provider = crate::backend::providers::get_provider(provider_id)?;
    let paths = agentmux_common::DataPaths::from_env()?;
    if let Some(bin) = crate::backend::cli_install::find_installed_for_provider(&paths, provider.id) {
        return Some(bin.to_string_lossy().to_string());
    }
    crate::server::cli_handlers::resolve_cli_on_path(provider.cli_command).await
}

/// A session title recovered from the session's recent activity, for an agent
/// that has none — called by the background sweep in
/// `backend::reactive::activity_watcher`, which writes an accepted result to
/// `term:ambient_summary` itself. Goes through the same Ambient Model Call gateway
/// (admission, cancellation-of-superseded, token accounting) under the distinct
/// `purpose::ACTIVITY_SUMMARY_PUSHED` purpose, so it never contends with the
/// pane's own title request. The prompt is the title prompt
/// (`build_session_title_from_activity_prompt`), not a "what is happening now"
/// summary: the result is shown as the session's title.
///
/// `generation` only needs to strictly increase across successive calls for
/// the *same* `block_id` — the sweep loop's tick counter is sufficient; it
/// doesn't need to correlate with the pull path's per-turn generation.
///
/// Returns `None` when there's nothing to summarize yet, the block/CLI path
/// isn't resolvable, this call was superseded, or the CLI failed — the
/// caller treats all of these as "no summary this tick."
pub(crate) async fn generate_recovered_title(
    mstore: &Store,
    filestore: &crate::backend::storage::filestore::FileStore,
    block_id: &str,
    generation: u64,
    word_target: u32,
) -> Option<Generated> {
    let word_target = word_target.max(3).min(20);

    // No concurrency permit here: the sweep in `activity_watcher` bounds itself.
    let slot = call::admit(AmbientCallKey::new(block_id, purpose::ACTIVITY_SUMMARY_PUSHED), generation, None).await?;

    let block: Block = mstore.get(block_id).ok().flatten()?;
    let Some(digest) = digest::read_recent_activity_digest(filestore, block_id) else {
        super::outcome::record(
            purpose::ACTIVITY_SUMMARY_PUSHED,
            block_id,
            super::outcome::Outcome::EmptyDigest,
            None,
        );
        return None;
    };
    let target = CliTarget::from_meta(&block.meta)?;

    let prompt = prompt::build_session_title_from_activity_prompt(word_target, &digest);
    finish(
        slot.run(&target, &prompt, |t| validate::accept_line(t, &validate::title_limits(word_target)))
            .await,
    )
}

/// Generate (and persist) a short activity summary for a definition whose
/// AgentPicker row has no structured `output.state.json` conversation
/// snapshot to preview — legacy rows predating snapshot persistence, per
/// `has_snapshot` in `agent_handlers::session::listrecentsessions`. Built
/// from the instance's raw terminal capture (the `"output"` filestore file,
/// written unconditionally by the CLI pipeline, independent of the newer
/// structured snapshot), reusing `read_recent_activity_digest` — the same
/// extraction `generate_pushed_activity_summary` above uses.
///
/// One-shot per definition, not per-turn: `generation` is always the
/// constant `1`, mirroring `generate_subagent_name`'s cache-once posture —
/// the caller only invokes this when `agent_activity_summary_get` found
/// nothing persisted yet, so there is no "newer turn" to supersede an
/// in-flight call here (unlike the pull/pushed activity-summary RPCs,
/// which regenerate every turn for a LIVE conversation).
///
/// CLI path resolution (reagent P1, PR #2786): the row shape this feature
/// actually targets is a CLOSED pane — `DeleteBlock`
/// (`sagas::delete_block::run`) removes the `Block` row entirely on pane
/// close while the instance row and raw filestore output survive. For such
/// rows there is no live block to read `cmd`/`cmd:env` meta from at all, so
/// this prefers the block record when it still exists (richer: carries the
/// per-block auth env the CLI needs) and falls back to
/// `resolve_provider_cli_path_readonly` + an EMPTY auth env otherwise —
/// best-effort: a provider that needs per-block injected credentials (no
/// ambient/global login available) fails the Haiku call cleanly (`None`,
/// same as any other unresolvable case here), not a wrong result.
///
/// Fire-and-forget by design: the caller (`listrecentsessions`) spawns this
/// in the background and does not await it inline. The row that triggered
/// generation still shows its existing fallback text on THIS response; on
/// success this broadcasts `agents:changed`, which `MyAgentsList.tsx`
/// already refetches on, picking up the now-persisted summary on the next
/// load. Returns `None` (nothing persisted, nothing broadcast) when there's
/// no raw output to summarize, the CLI path isn't resolvable, this call was
/// superseded/capped, or the CLI failed — the caller treats all of these as
/// "still nothing to show," not an error.
pub(crate) async fn generate_definition_activity_summary(
    mstore: &Store,
    filestore: &crate::backend::storage::filestore::FileStore,
    broker: &Arc<crate::backend::mps::Broker>,
    definition_id: &str,
    block_id: &str,
    provider_id: &str,
) -> Option<Generated> {
    // Background-call semaphore, not `pull_call_semaphore()` — see
    // `limits::definition_summary_semaphore`'s own doc comment.
    let slot = call::admit(
        AmbientCallKey::new(definition_id.to_string(), purpose::DEFINITION_SUMMARY),
        1,
        Some(limits::definition_summary_semaphore()),
    )
    .await?;

    let digest = digest::read_recent_activity_digest(filestore, block_id)?;

    let target = match mstore.get::<Block>(block_id) {
        Ok(Some(block)) => match CliTarget::from_meta(&block.meta) {
            Some(target) => target,
            None => provider_fallback_target(provider_id).await?,
        },
        _ => provider_fallback_target(provider_id).await?,
    };

    let prompt = prompt::build_definition_summary_prompt(&digest);
    let generated = finish(
        slot.run(&target, &prompt, |t| validate::accept_line(t, &validate::PREVIEW))
            .await,
    )?;
    let Some(summary) = generated.text.clone() else {
        return Some(generated);
    };

    let now = agentmux_common::time::now_ms();
    match mstore.agent_activity_summary_set(definition_id, &summary, now) {
        Ok(()) => {
            broker.publish(crate::backend::mps::MuxEvent {
                event: "agents:changed".to_string(),
                scopes: vec![],
                sender: String::new(),
                persist: 0,
                data: None,
            });
        }
        Err(e) => {
            // reagent P2, PR #2786: the caller's definition_summary_attempted()
            // gate already permanently claims definition_id before spawning —
            // a persistence failure here silently and permanently discards a
            // successfully generated (and billed) summary with no diagnostic
            // trail otherwise, unlike every other fallible store call this PR
            // touches (agent_activity_summary_get's error path logs).
            tracing::warn!(
                error = %e,
                definition_id = %definition_id,
                "generate_definition_activity_summary: agent_activity_summary_set \
                 failed — a successfully generated summary was discarded"
            );
        }
    }

    Some(generated)
}

/// The provider's own CLI with an EMPTY auth env, for a block that no longer
/// exists or names no CLI. `None` when the provider's CLI isn't installed.
async fn provider_fallback_target(provider_id: &str) -> Option<CliTarget> {
    let cli_path = resolve_provider_cli_path_readonly(provider_id).await?;
    Some(CliTarget { cli_path, meta: MetaMapType::new() })
}

/// Generate (or return the already-cached) concise Haiku display name for a
/// subagent. Called on-demand the first time a client expands that
/// subagent's row in the Swarm view — see `("subagent", "GenerateName")` in
/// `server::service::misc`. Subagents have no `Block`/meta of their own, so
/// this borrows the parent block's CLI path + auth env, and reads the
/// subagent's own initial task prompt directly off its JSONL (available
/// immediately even for a still-running subagent, unlike a transcript
/// summary which needs output to summarize).
///
/// Returns `None` when there's nothing to name (unknown subagent, no task
/// prompt on the first JSONL line, parent block unresolvable, or the call
/// was superseded/capped/failed) — callers should treat that as "leave the
/// row showing its slug/id fallback," not an error. A cache hit (name
/// already generated) returns the cached name with `tokens: None` — there's
/// no new spend to report.
pub(crate) async fn generate_subagent_name(
    mstore: &Store,
    subagent_watcher: &Arc<crate::backend::subagent_watcher::SubagentWatcher>,
    agent_id: &str,
    semaphore: &'static tokio::sync::Semaphore,
) -> Option<Generated> {
    let info = subagent_watcher.get_info(agent_id)?;
    if let Some(existing) = info.display_name {
        return Some(Generated { text: Some(existing), tokens: None });
    }

    // Concurrency cap — `pull_call_semaphore()` for the live on-click path
    // (a user rapidly expanding several subagent rows shouldn't spawn
    // unbounded concurrent Haiku CLIs either), `backlog_naming_semaphore()`
    // for the bounded backfill pass — see each call site.
    let slot = call::admit(
        AmbientCallKey::new(agent_id.to_string(), purpose::SUBAGENT_NAME),
        1,
        Some(semaphore),
    )
    .await?;

    let task_prompt = crate::backend::subagent_watcher::read_task_prompt(&info.jsonl_path)?;
    let block: Block = mstore.get(&info.parent_block_id).ok().flatten()?;
    let target = CliTarget::from_meta(&block.meta)?;

    let prompt = prompt::build_subagent_name_prompt(&task_prompt);
    let generated = finish(
        slot.run(&target, &prompt, |t| validate::accept_line(t, &validate::NAME))
            .await,
    )?;
    if let Some(name) = &generated.text {
        subagent_watcher.set_display_name(agent_id, name);
    }
    Some(generated)
}

/// Generate the one Haiku display name for a Workflow-kind dispatch,
/// eagerly, the first time its first member is observed live (never called
/// for a Solo dispatch — a Solo dispatch's name IS its one member's
/// `display_name`, already covered by `generate_subagent_name`; never called
/// during cold-backfill replay — see `subagent_watcher.rs`'s
/// `trigger_eager_naming`/`process_jsonl_change`'s `live` gate).
///
/// A workflow has no single task prompt the way a solo call does (members
/// can have different prompts) — this reads `first_member_agent_id`'s own
/// task prompt via the same `read_task_prompt()` `generate_subagent_name`
/// uses, on the resolved design basis that the first member's prompt is a
/// reasonable stand-in for the whole batch (SPEC §3 — not a perfect
/// representation of every member's task, an accepted v1 trade-off).
///
/// Otherwise mirrors `generate_subagent_name`'s admission/semaphore/prompt/
/// block-resolve/haiku-call shape exactly — no cache-hit short-circuit here
/// (unlike that function): this is only ever called once per dispatch,
/// already guarded by `naming_triggered` at the call site, so a cache check
/// would be dead code, not a real fast path.
pub(crate) async fn generate_dispatch_name(
    mstore: &Store,
    subagent_watcher: &Arc<crate::backend::subagent_watcher::SubagentWatcher>,
    dispatch_id: &str,
    first_member_agent_id: &str,
    semaphore: &'static tokio::sync::Semaphore,
) -> Option<Generated> {
    let info = subagent_watcher.get_info(first_member_agent_id)?;

    // Concurrency cap — see `generate_subagent_name`'s matching comment
    // above; same two possible callers, same two possible semaphores.
    let slot = call::admit(
        AmbientCallKey::new(dispatch_id.to_string(), purpose::DISPATCH_NAME),
        1,
        Some(semaphore),
    )
    .await?;

    let task_prompt = crate::backend::subagent_watcher::read_task_prompt(&info.jsonl_path)?;
    let block: Block = mstore.get(&info.parent_block_id).ok().flatten()?;
    let target = CliTarget::from_meta(&block.meta)?;

    let prompt = prompt::build_dispatch_name_prompt(&task_prompt);
    let generated = finish(
        slot.run(&target, &prompt, |t| validate::accept_line(t, &validate::NAME))
            .await,
    )?;
    if let Some(name) = &generated.text {
        subagent_watcher.set_dispatch_name(dispatch_id, name);
    }
    Some(generated)
}

/// Generate a short user-facing line narrating an autonomous action.
///
/// Best-effort by construction — every failure path returns `None` and the
/// caller simply says nothing. A UI state change must NEVER be gated on this:
/// if Haiku is slow, capped out, or absent, the thing being narrated still
/// happened and the UI must already reflect it.
pub(crate) async fn generate_ambient_narration(
    mstore: &Store,
    block_id: &str,
    event_id: &str,
    generation: u64,
    kind: &str,
    context: &str,
) -> Option<String> {
    let prompt = prompt::narration_prompt(kind, context)?;

    // Keyed on the EVENT, not the block. `admit()` cancels the prior in-flight
    // call for a key as soon as a newer generation arrives — correct for the
    // summary callers this is otherwise modelled on, where only the latest
    // result is ever shown, and wrong here. Each narrated event is independent
    // and all of them should arrive: two Bash calls backgrounded moments apart
    // in one block are two separate facts, and `useAmbientNarration` retains
    // several precisely because it expects them. Keying on the block alone let
    // the second silently cancel the first, whose Haiku call then returned
    // "cancelled" and was swallowed. (reagent P1 on #3169.)
    let slot = call::admit(
        AmbientCallKey::new(format!("{block_id}:{event_id}"), purpose::NARRATION),
        generation,
        Some(limits::narration_semaphore()),
    )
    .await?;

    let block: Block = mstore.get(block_id).ok().flatten()?;
    let target = CliTarget::from_meta(&block.meta)?;
    let (text, _tokens) = slot
        .run(&target, &prompt, |t| validate::accept_line(t, &validate::NARRATION))
        .await
        .into_option()?;
    Some(text)
}

/// reagent P1, PR #2786: `generate_definition_activity_summary` falls back
/// to this resolver when the instance's `Block` row is gone (the closed-
/// pane case this feature actually targets). Only the cheap, deterministic
/// "unknown provider" early return is exercised here — the filesystem/PATH
/// probing branches depend on this machine's actual CLI install state and
/// aren't meaningfully unit-testable without mocking the filesystem.
#[cfg(test)]
mod resolve_provider_cli_path_readonly_tests {
    use super::*;

    #[tokio::test]
    async fn unknown_provider_returns_none_without_touching_the_filesystem() {
        assert!(resolve_provider_cli_path_readonly("not-a-real-provider-xyz").await.is_none());
    }
}

#[cfg(test)]
mod narration_key_tests {
    use super::*;

    #[test]
    fn two_events_in_one_block_do_not_cancel_each_other() {
        // The regression reagent caught on #3169. admit() cancels the prior
        // in-flight call for a KEY, so keying narration on the block alone made
        // a second backgrounded command in the same turn silently kill the
        // first one's narration. Distinct event ids must be distinct keys.
        use crate::ambient::{gateway, Admission};

        let first = AmbientCallKey::new("block-x:toolu_1", purpose::NARRATION);
        let second = AmbientCallKey::new("block-x:toolu_2", purpose::NARRATION);
        assert_ne!(first, second, "distinct events must not share a gateway key");

        let g1 = match gateway().admit(first, 1) {
            Admission::Proceed(g) => g,
            Admission::StaleOnArrival => panic!("first should be admitted"),
        };
        let cancel1 = g1.cancellation();
        let _g2 = match gateway().admit(second, 2) {
            Admission::Proceed(g) => g,
            Admission::StaleOnArrival => panic!("second should be admitted"),
        };
        // The second admission must leave the first alone.
        assert!(!cancel1.is_cancelled(), "a sibling narration cancelled the first");
    }
}

#[cfg(test)]
mod finish_tests {
    use super::*;

    #[test]
    fn a_failed_or_superseded_call_is_not_a_finished_attempt() {
        let reply = Reply { text: String::new(), tokens: None, error: Some("cancelled".into()) };
        assert!(finish(reply).is_none());
    }

    #[test]
    fn a_rejected_reply_is_a_finished_attempt_that_keeps_its_spend() {
        let tokens = Some(TokenCounts::default());
        let reply = Reply { text: String::new(), tokens: tokens.clone(), error: None };
        let generated = finish(reply).expect("the CLI ran");
        assert!(generated.text.is_none());
        assert!(generated.tokens.is_some());
    }

    #[test]
    fn a_usable_reply_carries_its_text_and_spend() {
        let reply = Reply { text: "Fix login".into(), tokens: Some(TokenCounts::default()), error: None };
        let generated = finish(reply).unwrap();
        assert_eq!(generated.text.as_deref(), Some("Fix login"));
        assert!(generated.tokens.is_some());
    }
}
