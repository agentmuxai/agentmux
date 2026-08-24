use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_session_activity_summary(engine, state);
    register_session_next_prompt_suggestion(engine, state);
    register_session_archive_handler(engine, state);
    register_session_restore_handler(engine, state);
    register_session_export_handler(engine, state);
}

fn register_session_archive_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_ARCHIVE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandSessionArchiveData = serde_json::from_value(data)
                    .map_err(|e| format!("session:archive: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, "session:archive");

                let archive_dir = session_archive::default_archive_dir()
                    .ok_or_else(|| "cannot determine home directory".to_string())?;

                let (archived_bytes, archived_at) = session_archive::archive_session_output(
                    &wstore,
                    &filestore,
                    &cmd.block_id,
                    &archive_dir,
                )?;

                Ok(Some(serde_json::to_value(&SessionArchiveResult {
                    block_id: cmd.block_id,
                    archived_bytes,
                    archived_at,
                }).unwrap()))
            })
        }),
    );
}

fn register_session_restore_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_RESTORE,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandSessionRestoreData = serde_json::from_value(data)
                    .map_err(|e| format!("session:restore: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, "session:restore");

                let restored_bytes = session_archive::restore_session_output(
                    &wstore,
                    &filestore,
                    &cmd.block_id,
                )?;

                Ok(Some(serde_json::to_value(&SessionRestoreResult {
                    block_id: cmd.block_id,
                    restored_bytes,
                }).unwrap()))
            })
        }),
    );
}

fn register_session_export_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_EXPORT,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandSessionExportData = serde_json::from_value(data)
                    .map_err(|e| format!("session:export: {e}"))?;

                tracing::info!(block_id = %cmd.block_id, "session:export");

                let (raw_bytes, line_count) = session_archive::read_session_output(
                    &wstore,
                    &filestore,
                    &cmd.block_id,
                )?;

                let byte_count = raw_bytes.len() as u64;
                let content = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);

                Ok(Some(serde_json::to_value(&SessionExportResult {
                    content,
                    line_count,
                    byte_count,
                }).unwrap()))
            })
        }),
    );
}

/// Max simultaneous Haiku CLI spawns across the two user-turn-triggered pull
/// RPCs (`session:activity_summary`, `session:next_prompt_suggestion`). The
/// Ambient Model Call gateway's `admit()` only dedupes/cancels *per key*
/// (per block+purpose) — with no cap across different blocks, many panes
/// finishing a turn around the same moment could each spawn their own Haiku
/// subprocess unbounded. Mirrors `activity_watcher.rs`'s
/// `MAX_CONCURRENT_SUMMARIES` cap on the separate pushed-summary sweep, but
/// kept as its own semaphore rather than shared with that one: a burst of
/// background sweep summaries should not queue behind, or block, a live
/// user-facing pane-header/ghost-text request, and vice versa.
const MAX_CONCURRENT_PULL_CALLS: usize = 2;

pub(crate) fn pull_call_semaphore() -> &'static tokio::sync::Semaphore {
    static SEM: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SEM.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_PULL_CALLS))
}

/// Ambient-call purpose tag for the per-turn activity summary. Also usable
/// as the token-usage/cost-dashboard category for this call site.
const AMBIENT_PURPOSE_ACTIVITY_SUMMARY: &str = "activity_summary";

/// Purpose tag for the background/pushed activity summary (the swarm-feed
/// sweep in `backend::reactive::activity_watcher`) — distinct from
/// `AMBIENT_PURPOSE_ACTIVITY_SUMMARY` so the two never contend for the same
/// Ambient Model Call gateway slot. A periodic background summary should not
/// cancel, or be cancelled by, a live user-facing pane-header request for
/// the same block.
const AMBIENT_PURPOSE_ACTIVITY_SUMMARY_PUSHED: &str = "activity_summary_pushed";

fn empty_summary_result() -> serde_json::Value {
    serde_json::to_value(&ActivitySummaryResult { summary: String::new(), tokens: None }).unwrap()
}

/// Pushed counterpart of `register_session_activity_summary`'s handler body —
/// callable directly (no RPC envelope) by the background sweep in
/// `backend::reactive::activity_watcher`. Goes through the same Ambient
/// Model Call gateway (admission, cancellation-of-superseded, token
/// accounting) under the distinct `_PUSHED` purpose above.
///
/// `generation` only needs to strictly increase across successive calls for
/// the *same* `block_id` — the sweep loop's tick counter is sufficient; it
/// doesn't need to correlate with the pull path's per-turn generation.
///
/// Returns `None` when there's nothing to summarize yet, the block/CLI path
/// isn't resolvable, this call was superseded, or the CLI failed — the
/// caller treats all of these as "no summary this tick."
pub(crate) async fn generate_pushed_activity_summary(
    wstore: &Store,
    filestore: &crate::backend::storage::filestore::FileStore,
    block_id: &str,
    generation: u64,
    word_target: u32,
) -> Option<(String, Option<crate::agents::TokenCounts>)> {
    let word_target = word_target.max(3).min(20);

    let key = crate::ambient::AmbientCallKey::new(block_id, AMBIENT_PURPOSE_ACTIVITY_SUMMARY_PUSHED);
    let guard = match crate::ambient::gateway().admit(key, generation) {
        crate::ambient::Admission::Proceed(guard) => guard,
        crate::ambient::Admission::StaleOnArrival => return None,
    };
    let cancel = guard.cancellation();

    let block: Block = wstore.get(block_id).ok().flatten()?;
    let extracted = read_recent_activity_digest(filestore, block_id)?;

    let cli_path = obj::meta_get_string(&block.meta, "cmd", "");
    if cli_path.is_empty() {
        return None;
    }

    let prompt = format!(
        "Summarize in {word_target} words or fewer what is currently being worked on. \
         Plain text only — no markdown, no code fences, no backticks, no quotes, \
         no punctuation, no preamble.\n\n\
         Recent activity:\n\n{extracted}"
    );

    let result = invoke_ambient_haiku_call(&cli_path, &prompt, &block.meta, cancel).await.ok();
    drop(guard);
    result.filter(|(summary, _)| !summary.is_empty())
}

/// Ambient-call purpose tag for the on-demand, once-per-definition activity
/// summary used as the AgentPicker's "My Agents" conversation-preview
/// fallback — see `generate_definition_activity_summary` below and
/// `db_agent_activity_summaries` (OBJECT_SCHEMA_VERSION v28).
const AMBIENT_PURPOSE_DEFINITION_SUMMARY: &str = "definition_summary";

/// Max simultaneous Haiku CLI spawns for definition-summary generation.
/// Deliberately its OWN semaphore, not `pull_call_semaphore()` (reagent P1,
/// PR #2786): that one is reserved for live, user-turn-triggered pull RPCs
/// specifically so background bursts don't queue behind or block them (see
/// its own doc comment above) — this call is background-triggered (a
/// `listrecentsessions` poll, not a direct user action), the same class as
/// `activity_watcher.rs`'s pushed-summary sweep, which likewise gets its
/// own dedicated semaphore rather than sharing this one. Capped at 1: this
/// is best-effort background fill-in, not latency-sensitive.
const MAX_CONCURRENT_DEFINITION_SUMMARIES: usize = 1;

fn definition_summary_semaphore() -> &'static tokio::sync::Semaphore {
    static SEM: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SEM.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_DEFINITION_SUMMARIES))
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
async fn resolve_provider_cli_path_readonly(provider_id: &str) -> Option<String> {
    const AGENTMUX_VERSION: &str = env!("CARGO_PKG_VERSION");
    let provider = crate::backend::providers::get_provider(provider_id)?;
    let paths = agentmux_common::DataPaths::from_env()?;
    let provider_dir = paths
        .home_dir
        .join("instances")
        .join(format!("v{AGENTMUX_VERSION}"))
        .join("cli")
        .join(provider.id);
    let npm_bin = if cfg!(windows) {
        provider_dir.join("node_modules").join(".bin").join(format!("{}.cmd", provider.cli_command))
    } else {
        provider_dir.join("node_modules").join(".bin").join(provider.cli_command)
    };
    if npm_bin.exists() {
        return Some(npm_bin.to_string_lossy().to_string());
    }
    crate::server::cli_handlers::resolve_cli_on_path(provider.cli_command).await
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
    wstore: &Store,
    filestore: &crate::backend::storage::filestore::FileStore,
    broker: &Arc<crate::backend::wps::Broker>,
    definition_id: &str,
    block_id: &str,
    provider_id: &str,
) -> Option<(String, Option<crate::agents::TokenCounts>)> {
    let key = crate::ambient::AmbientCallKey::new(definition_id.to_string(), AMBIENT_PURPOSE_DEFINITION_SUMMARY);
    let guard = match crate::ambient::gateway().admit(key, 1) {
        crate::ambient::Admission::Proceed(guard) => guard,
        crate::ambient::Admission::StaleOnArrival => return None,
    };
    let cancel = guard.cancellation();

    // Background-call semaphore, not `pull_call_semaphore()` — see
    // `definition_summary_semaphore`'s own doc comment above.
    let permit = tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        permit = definition_summary_semaphore().acquire() => permit.ok(),
    };
    let Some(_permit) = permit else {
        drop(guard);
        return None;
    };

    let Some(digest) = read_recent_activity_digest(filestore, block_id) else {
        drop(guard);
        return None;
    };

    let (cli_path, meta): (String, MetaMapType) = match wstore.get::<Block>(block_id) {
        Ok(Some(block)) => {
            let p = obj::meta_get_string(&block.meta, "cmd", "");
            if p.is_empty() {
                let Some(p) = resolve_provider_cli_path_readonly(provider_id).await else {
                    drop(guard);
                    return None;
                };
                (p, MetaMapType::new())
            } else {
                (p, block.meta.clone())
            }
        }
        _ => {
            let Some(p) = resolve_provider_cli_path_readonly(provider_id).await else {
                drop(guard);
                return None;
            };
            (p, MetaMapType::new())
        }
    };

    let prompt = format!(
        "Summarize in 12 words or fewer what this conversation/session was \
         about, based on the raw terminal output below. Plain text only — \
         no markdown, no code fences, no backticks, no quotes, no \
         punctuation at the end, no preamble.\n\n\
         Recent activity:\n\n{digest}"
    );

    let result = invoke_ambient_haiku_call(&cli_path, &prompt, &meta, cancel).await.ok();
    drop(guard);

    let (summary, tokens) = result?;
    let summary = summary.trim().to_string();
    if summary.is_empty() {
        return None;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    match wstore.agent_activity_summary_set(definition_id, &summary, now) {
        Ok(()) => {
            broker.publish(crate::backend::wps::WaveEvent {
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

    Some((summary, tokens))
}

/// Ambient-call purpose tag for the on-demand subagent display name (see
/// `generate_subagent_name`). One-shot per subagent — `generation` is always
/// the constant `1` since a name, once generated, is cached on
/// `SubAgent.display_name` and never regenerated; there is no "newer
/// turn" to supersede an in-flight naming call the way there is for the
/// per-turn pull RPCs above.
const AMBIENT_PURPOSE_SUBAGENT_NAME: &str = "subagent_name";

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
    wstore: &Store,
    subagent_watcher: &Arc<crate::backend::subagent_watcher::SubagentWatcher>,
    agent_id: &str,
) -> Option<(String, Option<crate::agents::TokenCounts>)> {
    let info = subagent_watcher.get_info(agent_id)?;
    if let Some(existing) = info.display_name {
        return Some((existing, None));
    }

    let key = crate::ambient::AmbientCallKey::new(agent_id.to_string(), AMBIENT_PURPOSE_SUBAGENT_NAME);
    let guard = match crate::ambient::gateway().admit(key, 1) {
        crate::ambient::Admission::Proceed(guard) => guard,
        crate::ambient::Admission::StaleOnArrival => return None,
    };
    let cancel = guard.cancellation();

    // Same cross-block concurrency cap as the pull RPCs above — a user
    // rapidly expanding several subagent rows shouldn't spawn unbounded
    // concurrent Haiku CLIs either.
    let permit = tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        permit = pull_call_semaphore().acquire() => permit.ok(),
    };
    let Some(_permit) = permit else {
        drop(guard);
        return None;
    };

    let Some(task_prompt) = crate::backend::subagent_watcher::read_task_prompt(&info.jsonl_path) else {
        drop(guard);
        return None;
    };

    let block: Block = match wstore.get(&info.parent_block_id) {
        Ok(Some(b)) => b,
        _ => {
            drop(guard);
            return None;
        }
    };
    let cli_path = obj::meta_get_string(&block.meta, "cmd", "");
    if cli_path.is_empty() {
        drop(guard);
        return None;
    }

    let prompt = format!(
        "Give a concise ~5-word name for this task. Plain text only — no markdown, \
         no code fences, no backticks, no punctuation, no quotes, no preamble. \
         Respond with just the name.\n\n\
         Task:\n\n{task_prompt}"
    );

    let result = invoke_ambient_haiku_call(&cli_path, &prompt, &block.meta, cancel).await.ok();
    drop(guard);

    let (name, tokens) = result?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return None;
    }

    subagent_watcher.set_display_name(agent_id, &name);
    Some((name, tokens))
}

/// Ambient-call purpose tag for eager Workflow-dispatch naming — distinct
/// from `AMBIENT_PURPOSE_SUBAGENT_NAME` so cost-dashboard tagging can tell
/// the two apart even though they share the same gateway/semaphore. See
/// docs/specs/SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md.
const AMBIENT_PURPOSE_DISPATCH_NAME: &str = "dispatch_name";

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
    wstore: &Store,
    subagent_watcher: &Arc<crate::backend::subagent_watcher::SubagentWatcher>,
    dispatch_id: &str,
    first_member_agent_id: &str,
) -> Option<(String, Option<crate::agents::TokenCounts>)> {
    let info = subagent_watcher.get_info(first_member_agent_id)?;

    let key = crate::ambient::AmbientCallKey::new(dispatch_id.to_string(), AMBIENT_PURPOSE_DISPATCH_NAME);
    let guard = match crate::ambient::gateway().admit(key, 1) {
        crate::ambient::Admission::Proceed(guard) => guard,
        crate::ambient::Admission::StaleOnArrival => return None,
    };
    let cancel = guard.cancellation();

    // Same cross-block concurrency cap as every other ambient caller — see
    // AMBIENT_PURPOSE_SUBAGENT_NAME's comment above.
    let permit = tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        permit = pull_call_semaphore().acquire() => permit.ok(),
    };
    let Some(_permit) = permit else {
        drop(guard);
        return None;
    };

    let Some(task_prompt) = crate::backend::subagent_watcher::read_task_prompt(&info.jsonl_path) else {
        drop(guard);
        return None;
    };

    let block: Block = match wstore.get(&info.parent_block_id) {
        Ok(Some(b)) => b,
        _ => {
            drop(guard);
            return None;
        }
    };
    let cli_path = obj::meta_get_string(&block.meta, "cmd", "");
    if cli_path.is_empty() {
        drop(guard);
        return None;
    }

    let prompt = format!(
        "Give a concise ~5-word name for this workflow batch, based on its \
         first task. Plain text only — no markdown, no code fences, no \
         backticks, no punctuation, no quotes, no preamble. Respond with \
         just the name.\n\n\
         Task:\n\n{task_prompt}"
    );

    let result = invoke_ambient_haiku_call(&cli_path, &prompt, &block.meta, cancel).await.ok();
    drop(guard);

    let (name, tokens) = result?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return None;
    }

    subagent_watcher.set_dispatch_name(dispatch_id, &name);
    Some((name, tokens))
}

fn register_session_activity_summary(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_ACTIVITY_SUMMARY,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandActivitySummaryData = serde_json::from_value(data)
                    .map_err(|e| format!("session:activity_summary: {e}"))?;

                // Admit through the Ambient Model Call gateway BEFORE doing any
                // work: a stale (superseded) request does zero FileStore reads
                // or prompt building, not just skips the CLI spawn. See
                // docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md.
                let key = crate::ambient::AmbientCallKey::new(
                    cmd.block_id.clone(),
                    AMBIENT_PURPOSE_ACTIVITY_SUMMARY,
                );
                let guard = match crate::ambient::gateway().admit(key, cmd.generation) {
                    crate::ambient::Admission::Proceed(guard) => guard,
                    crate::ambient::Admission::StaleOnArrival => {
                        return Ok(Some(empty_summary_result()));
                    }
                };
                let cancel = guard.cancellation();

                // Cap concurrent Haiku spawns across all blocks — see
                // MAX_CONCURRENT_PULL_CALLS. Raced against cancellation so a
                // request superseded while queued for a permit never spawns
                // the CLI at all.
                let permit = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => None,
                    permit = pull_call_semaphore().acquire() => permit.ok(),
                };
                let Some(_permit) = permit else {
                    drop(guard);
                    return Ok(Some(empty_summary_result()));
                };

                let word_target = cmd.word_target.unwrap_or(7).max(3).min(20);

                let block: Block = wstore
                    .get(&cmd.block_id)
                    .map_err(|e| format!("session:activity_summary: {e}"))?
                    .ok_or_else(|| format!("BLOCK_NOT_FOUND: {}", cmd.block_id))?;

                // The user's newest message, verbatim — the frontend passes this
                // directly from the just-submitted TurnStart content, so the
                // common case needs no FileStore read at all. Falls back to the
                // old tail-digest extraction only when the caller didn't supply
                // one (e.g. an older frontend build), so the endpoint degrades
                // gracefully instead of going silent. See
                // docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md.
                let user_message = cmd
                    .user_message
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .or_else(|| read_recent_activity_digest(&filestore, &cmd.block_id));

                let current_title = obj::meta_get_string(&block.meta, "term:ambient_summary", "");

                // Nothing to anchor a title on AND nothing new to evaluate —
                // matches the old digest-empty early return.
                if user_message.is_none() && current_title.is_empty() {
                    return Ok(Some(empty_summary_result()));
                }

                let cli_path = obj::meta_get_string(&block.meta, "cmd", "");
                if cli_path.is_empty() {
                    tracing::debug!(block_id = %cmd.block_id, "session:activity_summary: no CLI path in meta");
                    return Ok(Some(empty_summary_result()));
                }

                let prompt = build_session_title_prompt(&current_title, user_message.as_deref(), word_target);

                let (summary, tokens) =
                    invoke_ambient_haiku_call(&cli_path, &prompt, &block.meta, cancel).await
                        .unwrap_or_else(|e| {
                            tracing::debug!(block_id = %cmd.block_id, error = %e, "session:activity_summary: CLI failed or was superseded");
                            (String::new(), None)
                        });

                // guard is held until here so the in-flight entry stays
                // registered (and cancellable by a newer request) for the
                // full duration of the CLI call.
                drop(guard);

                // The frontend writes `term:ambient_summary` after receiving this
                // response so it can discard results from turns that were
                // superseded before they returned (belt-and-suspenders on top of
                // the gateway's own cancellation).
                Ok(Some(serde_json::to_value(&ActivitySummaryResult { summary, tokens }).unwrap()))
            })
        }),
    );
}

/// Ambient-call purpose tag for the ghost-text next-prompt suggestion. See
/// docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md.
const AMBIENT_PURPOSE_NEXT_PROMPT_SUGGESTION: &str = "next_prompt_suggestion";

fn empty_suggestion_result() -> serde_json::Value {
    serde_json::to_value(&NextPromptSuggestionResult { suggestion: String::new(), tokens: None }).unwrap()
}

fn register_session_next_prompt_suggestion(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    let filestore = state.filestore.clone();

    engine.register_handler(
        COMMAND_SESSION_NEXT_PROMPT_SUGGESTION,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let filestore = filestore.clone();
            Box::pin(async move {
                let cmd: CommandNextPromptSuggestionData = serde_json::from_value(data)
                    .map_err(|e| format!("session:next_prompt_suggestion: {e}"))?;

                // Same admission discipline as activity_summary — see that
                // handler's comment. Ghost text has a sharper failure mode
                // than the read-only summary (a stale suggestion can put
                // words in the user's mouth), so admitting before any work
                // matters just as much here.
                let key = crate::ambient::AmbientCallKey::new(
                    cmd.block_id.clone(),
                    AMBIENT_PURPOSE_NEXT_PROMPT_SUGGESTION,
                );
                let guard = match crate::ambient::gateway().admit(key, cmd.generation) {
                    crate::ambient::Admission::Proceed(guard) => guard,
                    crate::ambient::Admission::StaleOnArrival => {
                        return Ok(Some(empty_suggestion_result()));
                    }
                };
                let cancel = guard.cancellation();

                // Same cross-block concurrency cap as activity_summary — the
                // two pull RPCs share MAX_CONCURRENT_PULL_CALLS so neither
                // alone can flood the machine with Haiku subprocesses.
                let permit = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => None,
                    permit = pull_call_semaphore().acquire() => permit.ok(),
                };
                let Some(_permit) = permit else {
                    drop(guard);
                    return Ok(Some(empty_suggestion_result()));
                };

                let block: Block = wstore
                    .get(&cmd.block_id)
                    .map_err(|e| format!("session:next_prompt_suggestion: {e}"))?
                    .ok_or_else(|| format!("BLOCK_NOT_FOUND: {}", cmd.block_id))?;

                let Some(extracted) = read_recent_activity_digest(&filestore, &cmd.block_id) else {
                    return Ok(Some(empty_suggestion_result()));
                };

                let cli_path = obj::meta_get_string(&block.meta, "cmd", "");
                if cli_path.is_empty() {
                    tracing::debug!(block_id = %cmd.block_id, "session:next_prompt_suggestion: no CLI path in meta");
                    return Ok(Some(empty_suggestion_result()));
                }

                let prompt = format!(
                    "Based on this recent activity, predict ONE short next instruction \
                     the user is likely to give to continue the work. Phrase it as a \
                     direct, imperative command — the way someone types a task into a \
                     prompt box, not casual chat. Do not start with conversational \
                     filler or throat-clearing (\"Yeah\", \"Sure\", \"Let's\", \"Go ahead \
                     and\", \"OK\", or similar) — begin directly with the action itself. \
                     For example, write \"Debug the blank preview bug next\", not \"Yeah \
                     let's debug the blank preview bug next\". Respond with just that \
                     instruction and nothing else — plain text only, no markdown, no code \
                     fences, no backticks, no quotes, no explanation, no preamble. If \
                     nothing plausible comes to mind, respond with an empty string.\n\n\
                     Recent activity:\n\n{extracted}"
                );

                let (suggestion, tokens) =
                    invoke_ambient_haiku_call(&cli_path, &prompt, &block.meta, cancel).await
                        .unwrap_or_else(|e| {
                            tracing::debug!(block_id = %cmd.block_id, error = %e, "session:next_prompt_suggestion: CLI failed or was superseded");
                            (String::new(), None)
                        });

                // guard is held until here — same rationale as activity_summary.
                drop(guard);

                Ok(Some(serde_json::to_value(&NextPromptSuggestionResult { suggestion, tokens }).unwrap()))
            })
        }),
    );
}

/// Build the session-goal-title prompt for `session:activity_summary`.
/// Explicitly PR-title-style and stability-biased: the previous "what is
/// currently being worked on" prompt regenerated from a blank slate every
/// call (no memory of the prior title, no access to the original ask), so
/// it thrashed between micro-steps instead of tracking the session's
/// overall goal. Extracted as a pure function so the prompt shape itself
/// (both fields embedded, correct fallback text) is unit-testable without
/// spinning up the full RPC handler. See
/// docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md.
fn build_session_title_prompt(current_title: &str, user_message: Option<&str>, word_target: u32) -> String {
    let current_title_display = if current_title.is_empty() { "(none yet)" } else { current_title };
    let user_message_display =
        user_message.unwrap_or("(no new message — re-evaluate from the title alone)");

    format!(
        "You maintain a short running TITLE for this work session, similar to a git \
         pull-request title — it describes the OVERALL GOAL of the session, not the \
         current micro-step or the most recent tool call.\n\n\
         Current title: {current_title_display}\n\n\
         The user just said:\n{user_message_display}\n\n\
         Decide: does this message represent a genuinely NEW or EXPANDED top-level \
         goal, or is it a continuation, follow-up, clarification, correction, or a \
         step within the SAME goal the current title already describes?\n\n\
         - If the current title still accurately describes the overall goal, repeat \
         it back EXACTLY, unchanged.\n\
         - Otherwise, output an updated title covering the (possibly still-in-progress) \
         overall goal, in {word_target} words or fewer.\n\n\
         Plain text only — no markdown, no code fences, no backticks, no quotes, \
         no punctuation, no preamble."
    )
}

/// Read the last 32 KB of a block's FileStore output, take the most recent
/// ~30 non-empty lines, and extract digest text from them (`extract_digest_text`).
/// Used directly by `next_prompt_suggestion`; `activity_summary` now only
/// falls back to this when the caller didn't supply `user_message` (see
/// `docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md`) —
/// the common case passes the literal just-submitted text instead. Returns
/// `None` when there's nothing usable — callers should return an empty
/// ambient result in that case without invoking the CLI.
///
/// EVENT_BLOCK_FILE events have persist: 0 so the ring buffer is always
/// empty — FileStore is the only reliable source. Tail-reading avoids
/// loading multi-MB output files on every turn; 32 KB comfortably covers
/// 30 stream-json lines.
fn read_recent_activity_digest(
    filestore: &crate::backend::storage::filestore::FileStore,
    block_id: &str,
) -> Option<String> {
    const TAIL_BYTES: i64 = 32 * 1024;
    let all_lines: Vec<String> = match filestore.stat(block_id, "output") {
        Ok(Some(ref wf)) if wf.size > 0 => {
            let tail_offset = (wf.size - TAIL_BYTES).max(0);
            match filestore.read_at(block_id, "output", tail_offset, TAIL_BYTES) {
                Ok((_, bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    text.lines()
                        .filter(|l| !l.trim().is_empty())
                        .map(|l| l.to_string())
                        .collect()
                }
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    };

    let n = all_lines.len();
    let start = n.saturating_sub(30);
    let window: Vec<&str> = all_lines[start..].iter().map(|s| s.as_str()).collect();
    if window.is_empty() {
        return None;
    }

    let extracted = extract_digest_text(&window);
    if extracted.is_empty() {
        return None;
    }
    Some(extracted)
}

/// Invoke the Claude CLI with Haiku model for a lightweight ambient call
/// (activity summary, ghost-text next-prompt suggestion, or any future
/// purpose routed through the Ambient Model Call gateway). Uses
/// `--model claude-haiku-4-5-20251001` and a 15s timeout.
///
/// `cancel` is this call's Ambient Model Call gateway cancellation token — if
/// a newer request for the same `(block_id, purpose)` key is admitted while
/// this one is still running, `cancel` fires and the child process is killed
/// immediately rather than left to run to completion (and keep burning
/// tokens) only to have its result discarded on arrival.
///
/// Returns the response text plus token usage parsed from the CLI's `result`
/// stream-json line (same `usage` shape the main turn pipeline parses —
/// see `agents::translator::claude::parse_usage`), so ambient calls are
/// never silently excluded from token accounting.
pub(crate) async fn invoke_ambient_haiku_call(
    cli_path: &str,
    prompt: &str,
    meta: &obj::MetaMapType,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<(String, Option<crate::agents::TokenCounts>), String> {
    let auth_env: std::collections::HashMap<String, String> = match meta.get("cmd:env") {
        Some(serde_json::Value::Object(obj_map)) => obj_map
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect(),
        _ => std::collections::HashMap::new(),
    };

    let mut child = crate::server::cli_handlers::make_cli_cmd(cli_path)
        .args(["-p", "--output-format", "stream-json", "--verbose",
               "--model", "claude-haiku-4-5-20251001"])
        .envs(&auth_env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn activity CLI: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(prompt.as_bytes()).await
            .map_err(|e| format!("activity CLI stdin write: {e}"))?;
        stdin.shutdown().await
            .map_err(|e| format!("activity CLI stdin shutdown: {e}"))?;
    }

    // `child.wait()` only borrows (unlike `wait_with_output()`, which
    // consumes `child` by value and would make the cancel branch's
    // `child.kill()` below impossible). Drain stdout concurrently via a
    // separate task — same rationale `wait_with_output()` itself uses
    // internally — so a chatty response can't fill the OS pipe buffer and
    // deadlock the child waiting to write while nothing is reading.
    let mut stdout_pipe = child.stdout.take()
        .ok_or_else(|| "activity CLI: no stdout pipe".to_string())?;
    let stdout_task = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf).await;
        buf
    });

    let status = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err("cancelled: superseded by a newer activity-summary request".to_string());
        }
        result = tokio::time::timeout(std::time::Duration::from_secs(15), child.wait()) => {
            result.map_err(|_| "activity CLI timed out after 15s".to_string())?
                .map_err(|e| format!("activity CLI wait: {e}"))?
        }
    };

    if !status.success() {
        return Err(format!("activity CLI exited with status {status}"));
    }

    let stdout_bytes = stdout_task.await
        .map_err(|e| format!("activity CLI stdout reader task: {e}"))?;
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let mut last_text = String::new();
    let mut tokens: Option<crate::agents::TokenCounts> = None;
    for line in stdout.lines() {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match val.get("type").and_then(|v| v.as_str()) {
            Some("assistant") => {
                if let Some(content) = val.get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                last_text = text.trim().to_string();
                            }
                        }
                    }
                }
            }
            Some("result") => {
                tokens = Some(crate::agents::translator::claude::parse_usage(val.get("usage")));
            }
            _ => {}
        }
    }

    let last_text = sanitize_ambient_text(&last_text);
    if last_text.is_empty() {
        return Err("no text in activity CLI response".to_string());
    }

    Ok((last_text, tokens))
}

/// Defends against the model wrapping its answer in markdown, or opening
/// with conversational filler, despite every ambient-call prompt asking for
/// a bare, direct line of plain text — instruction-following isn't
/// guaranteed. An unwrapped fence is what produces a literal
/// ` ``` `/newline/` ``` ` blob on a UI surface that renders this text
/// verbatim (e.g. the ghost-text composer placeholder, which has no
/// markdown renderer). Filler ("Yeah, let's fix the bug" instead of "Fix
/// the bug") is a separate readability problem the prompt alone can't fully
/// prevent — same "prompt nudge + reliable sanitizer" split this project
/// already uses for the fence/quote case. Applied once here so every
/// current and future `invoke_ambient_haiku_call` caller is covered, not
/// just the one that first surfaced each bug.
///
/// Strips a wrapping code fence, wrapping quote characters, and a leading
/// conversational preamble, repeatedly (a model can combine all three —
/// e.g. a quoted, filler-prefixed sentence), then trims. A result left with
/// nothing but backticks (e.g. an empty fence) collapses to "" so callers'
/// existing empty-string handling (skip writing ghost text / filter out the
/// summary) covers it without any caller-side change.
fn sanitize_ambient_text(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for _ in 0..4 {
        let before = s.clone();
        if let Some(inner) = strip_wrapping_fence(&s) {
            s = inner;
        }
        s = s
            .trim_matches(|c: char| {
                matches!(c, '"' | '\'' | '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}')
            })
            .trim()
            .to_string();
        if let Some(rest) = strip_conversational_preamble(&s) {
            s = rest;
        }
        if s == before {
            break;
        }
    }
    if !s.is_empty() && s.chars().all(|c| c == '`') {
        s.clear();
    }
    s
}

/// Strip a wrapping code fence (triple-backtick, optionally with a language
/// tag on the opening line, or a single inline backtick) if the *entire*
/// string is wrapped — a fence-like substring embedded mid-sentence is left
/// alone. Returns the un-fenced inner text (not yet trimmed of quotes).
fn strip_wrapping_fence(s: &str) -> Option<String> {
    let s = s.trim();
    for fence in ["```", "`"] {
        if s.len() >= fence.len() * 2 && s.starts_with(fence) && s.ends_with(fence) {
            let mut inner = &s[fence.len()..s.len() - fence.len()];
            if fence == "```" {
                if let Some(nl) = inner.find('\n') {
                    inner = &inner[nl + 1..];
                }
            }
            return Some(inner.trim().to_string());
        }
    }
    None
}

/// Conversational-filler openers a model reaches for despite being told to
/// respond with a bare, direct instruction — e.g. "Yeah, let's fix the
/// bug" instead of "Fix the bug". Matched case-insensitively against the
/// very start of the string only (a "let's" appearing mid-sentence is left
/// alone — this strips openers, not arbitrary word choice). Longer/more
/// specific entries first so e.g. "let's go ahead and " matches whole
/// rather than the shorter "let's " eating only part of it.
const CONVERSATIONAL_PREAMBLES: &[&str] = &[
    "let's go ahead and ",
    "lets go ahead and ",
    "yeah, let's ",
    "yeah let's ",
    "yeah, lets ",
    "yeah lets ",
    "sure, let's ",
    "sure let's ",
    "ok, let's ",
    "okay, let's ",
    "alright, let's ",
    "go ahead and ",
    "let's ",
    "lets ",
    "sure, i'll ",
    "sure, i will ",
    "sure, ",
    "yeah, ",
    "yeah ",
    "ok, ",
    "okay, ",
    "alright, ",
    "i'll ",
    "i will ",
    "i should ",
    "we should ",
    "next up, ",
    "next, ",
];

/// Strip one leading conversational-filler opener (see
/// `CONVERSATIONAL_PREAMBLES`) and re-capitalize the new first letter, so
/// "Yeah, let's debug the bug" becomes "Debug the bug" — matching the
/// direct-instruction register every ambient-call prompt asks for. Returns
/// `None` if no known opener matches (left alone rather than guessed at —
/// this list is deliberately not exhaustive NLP-style filler detection,
/// just the handful of openers a model actually reaches for here).
fn strip_conversational_preamble(s: &str) -> Option<String> {
    for prefix in CONVERSATIONAL_PREAMBLES {
        if let Some(byte_len) = case_insensitive_prefix_byte_len(s, prefix) {
            let rest = &s[byte_len..];
            let mut chars = rest.chars();
            return Some(match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            });
        }
    }
    None
}

/// Byte length in `s` of a prefix that case-insensitively matches `prefix`
/// (always plain ASCII lowercase), or `None` if `s` doesn't start with it.
/// Walks `s`'s own char boundaries rather than slicing by an offset computed
/// against a separately-lowercased copy of `s` — a char's lowercase mapping
/// can change UTF-8 byte length (e.g. the Kelvin sign 'K' -> 'k') or even
/// character count (e.g. Turkish 'İ' -> "i̇"), which would otherwise misalign
/// the offset or land it off a char boundary and panic on slice.
fn case_insensitive_prefix_byte_len(s: &str, prefix: &str) -> Option<usize> {
    let mut prefix_chars = prefix.chars().peekable();
    for (byte_idx, c) in s.char_indices() {
        if prefix_chars.peek().is_none() {
            return Some(byte_idx);
        }
        for lc in c.to_lowercase() {
            if prefix_chars.next_if_eq(&lc).is_none() {
                return None;
            }
        }
    }
    if prefix_chars.peek().is_none() {
        Some(s.len())
    } else {
        None
    }
}

/// Extract meaningful text from raw stream-json lines for digest summarization.
/// Skips system/result events and raw stream_event deltas; extracts assistant text
/// and tool call summaries.
pub(super) fn extract_digest_text(lines: &[&str]) -> String {
    let mut parts: Vec<String> = Vec::new();

    for line in lines {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else { continue };

        let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match msg_type {
            "assistant" => {
                if let Some(content) = val.get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if btype == "text" {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    parts.push(format!("[assistant] {}", trimmed));
                                }
                            }
                        } else if btype == "tool_use" {
                            let tool_name = block.get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            parts.push(format!("[tool] {}", tool_name));
                        }
                    }
                }
            }
            "user" => {
                if let Some(content) = val.get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if btype == "tool_result" {
                            let is_error = block.get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if is_error {
                                let err_text = block.get("content")
                                    .and_then(|c| c.as_str())
                                    .unwrap_or("(error)")
                                    .chars().take(120).collect::<String>();
                                parts.push(format!("[error] {}", err_text));
                            }
                        } else if btype == "text" {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    parts.push(format!("[user] {}", trimmed));
                                }
                            }
                        }
                    }
                }
            }
            "result" => {
                if let Some(cost) = val.get("total_cost_usd").and_then(|v| v.as_f64()) {
                    if let Some(turns) = val.get("num_turns").and_then(|v| v.as_u64()) {
                        parts.push(format!("[summary] {} turns, ${:.4} total cost", turns, cost));
                    }
                }
            }
            // Skip: system, stream_event (deltas), rate_limit_event
            _ => {}
        }
    }

    parts.join("\n")
}

#[cfg(test)]
mod build_session_title_prompt_tests {
    use super::*;

    #[test]
    fn embeds_both_fields_when_present() {
        let prompt = build_session_title_prompt("invert user input styling", Some("also fix the lint warning"), 7);
        assert!(prompt.contains("Current title: invert user input styling"));
        assert!(prompt.contains("The user just said:\nalso fix the lint warning"));
        assert!(prompt.contains("in 7 words or fewer"));
    }

    #[test]
    fn falls_back_to_none_yet_for_an_empty_current_title() {
        let prompt = build_session_title_prompt("", Some("build a login page"), 7);
        assert!(prompt.contains("Current title: (none yet)"));
    }

    #[test]
    fn falls_back_to_a_placeholder_for_a_missing_user_message() {
        let prompt = build_session_title_prompt("invert user input styling", None, 7);
        assert!(prompt.contains("The user just said:\n(no new message — re-evaluate from the title alone)"));
    }

    #[test]
    fn instructs_stability_over_regeneration() {
        // The core behavior change: the prompt must explicitly bias toward
        // KEEPING the current title, not regenerating fresh every call —
        // this is what the old "what is currently being worked on" prompt
        // never asked for. docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md.
        let prompt = build_session_title_prompt("invert user input styling", Some("continue"), 7);
        assert!(prompt.contains("repeat it back EXACTLY, unchanged"));
        assert!(prompt.contains("OVERALL GOAL"));
        assert!(!prompt.contains("what is currently being worked on"));
    }
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
mod pull_call_semaphore_tests {
    use super::*;

    /// The two pull RPCs share one process-wide cap: once
    /// MAX_CONCURRENT_PULL_CALLS permits are held, a further non-blocking
    /// acquire must fail rather than let a third Haiku CLI spawn through.
    #[tokio::test]
    async fn caps_at_max_concurrent_pull_calls() {
        let sem = pull_call_semaphore();
        let mut held = Vec::new();
        for _ in 0..MAX_CONCURRENT_PULL_CALLS {
            held.push(sem.try_acquire().expect("permit within the cap should be available"));
        }
        assert!(
            sem.try_acquire().is_err(),
            "a permit beyond MAX_CONCURRENT_PULL_CALLS must not be granted"
        );

        // Releasing one frees a slot for the next caller.
        held.pop();
        assert!(sem.try_acquire().is_ok(), "releasing a permit must free a slot");
    }
}

#[cfg(test)]
mod sanitize_ambient_text_tests {
    use super::*;

    #[test]
    fn passes_plain_text_through_unchanged() {
        assert_eq!(sanitize_ambient_text("Run the tests"), "Run the tests");
    }

    #[test]
    fn strips_a_triple_backtick_fence() {
        assert_eq!(sanitize_ambient_text("```\nRun the tests\n```"), "Run the tests");
    }

    #[test]
    fn strips_a_fence_with_a_language_tag() {
        assert_eq!(sanitize_ambient_text("```text\nRun the tests\n```"), "Run the tests");
    }

    #[test]
    fn empty_fence_collapses_to_empty_string() {
        assert_eq!(sanitize_ambient_text("```\n```"), "");
        assert_eq!(sanitize_ambient_text("``````"), "");
    }

    #[test]
    fn strips_wrapping_single_backticks() {
        assert_eq!(sanitize_ambient_text("`Run the tests`"), "Run the tests");
    }

    #[test]
    fn strips_wrapping_quotes() {
        assert_eq!(sanitize_ambient_text("\"Run the tests\""), "Run the tests");
        assert_eq!(sanitize_ambient_text("'Run the tests'"), "Run the tests");
        assert_eq!(sanitize_ambient_text("\u{201C}Run the tests\u{201D}"), "Run the tests");
    }

    #[test]
    fn strips_nested_fence_and_quotes() {
        assert_eq!(sanitize_ambient_text("```\n\"Run the tests\"\n```"), "Run the tests");
    }

    #[test]
    fn leaves_an_embedded_fence_like_substring_alone() {
        let s = "Run `npm test` next";
        assert_eq!(sanitize_ambient_text(s), s);
    }

    #[test]
    fn lone_backticks_with_no_content_collapse_to_empty() {
        assert_eq!(sanitize_ambient_text("```"), "");
    }

    #[test]
    fn strips_yeah_lets_preamble() {
        assert_eq!(
            sanitize_ambient_text("Yeah, let's debug the blank preview bug next"),
            "Debug the blank preview bug next"
        );
        assert_eq!(sanitize_ambient_text("Yeah let's fix the login bug"), "Fix the login bug");
    }

    #[test]
    fn strips_go_ahead_and_preamble() {
        assert_eq!(
            sanitize_ambient_text("Go ahead and add tests for the parser"),
            "Add tests for the parser"
        );
    }

    #[test]
    fn strips_sure_ok_alright_preamble() {
        assert_eq!(sanitize_ambient_text("Sure, fix the typo"), "Fix the typo");
        assert_eq!(sanitize_ambient_text("OK, run the tests"), "Run the tests");
        assert_eq!(sanitize_ambient_text("Alright, let's ship it"), "Ship it");
    }

    #[test]
    fn strips_preamble_from_inside_quotes_and_fences() {
        assert_eq!(sanitize_ambient_text("\"Yeah, let's fix the bug\""), "Fix the bug");
        assert_eq!(sanitize_ambient_text("```\nYeah, let's fix the bug\n```"), "Fix the bug");
    }

    #[test]
    fn leaves_a_mid_sentence_lets_alone() {
        let s = "Check whether the retry logic still lets errors through";
        assert_eq!(sanitize_ambient_text(s), s);
    }

    #[test]
    fn a_filler_word_with_no_trailing_content_is_left_alone() {
        // "Yeah," alone (no instruction after it) doesn't match any
        // CONVERSATIONAL_PREAMBLES entry — they all require trailing
        // content after the opener, matching the "strip openers, not
        // guess at degenerate whole-string filler" scope this function
        // documents.
        assert_eq!(sanitize_ambient_text("Yeah,"), "Yeah,");
    }

    #[test]
    fn preamble_strip_handles_byte_length_changing_case_folding() {
        // U+212A KELVIN SIGN lowercases to ASCII 'k' (3 bytes -> 1 byte). If the
        // preamble strip computed its slice offset from a separately-lowercased
        // copy of the string instead of walking the original string's own char
        // boundaries, this would misalign the slice (or panic on a non-char-
        // boundary index) instead of matching "ok, " normally.
        let s = "O\u{212A}, run the tests";
        assert_eq!(sanitize_ambient_text(s), "Run the tests");
    }

    #[test]
    fn preamble_strip_handles_char_count_changing_case_folding() {
        // U+0130 LATIN CAPITAL LETTER I WITH DOT ABOVE lowercases to "i̇" (2
        // chars) under Rust's default Unicode case folding. This does not
        // match any configured preamble, but must not panic or corrupt the
        // string while failing to match.
        let s = "\u{0130} think we should refactor this";
        assert_eq!(sanitize_ambient_text(s), s);
    }
}
