use super::*;
use crate::ambient;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_session_activity_summary(engine, state);
    register_session_next_prompt_suggestion(engine, state);
    register_session_resume_preflight_handler(engine, state);
    register_session_archive_handler(engine, state);
    register_session_restore_handler(engine, state);
    register_session_export_handler(engine, state);
}

/// `session:resume_preflight` — read-only, mutates nothing, spawns nothing.
///
/// The pane calls this on mount so it can say whether the conversation it's
/// displaying will actually be continued, instead of the user finding out by
/// typing and watching the transcript clear
/// (`crate::backend::resume_preflight`'s module doc).
fn register_session_resume_preflight_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    // Needed to resolve an identity-bound pane's REAL config dir — see
    // `preflight_input_from_meta`.
    let id_store = state.id_store.clone();
    let identity_store = state.identity_store.clone();
    // The pane's own transcript, for the session its rendered history belongs to.
    let filestore = state.filestore.clone();

    engine.register_typed(
        COMMAND_SESSION_RESUME_PREFLIGHT,
        move |cmd: CommandSessionResumePreflightData, _ctx| {
            let mstore = mstore.clone();
            let id_store = id_store.clone();
            let identity_store = identity_store.clone();
            let filestore = filestore.clone();
            async move {

                let block = mstore
                    .must_get::<Block>(&cmd.block_id)
                    .map_err(|e| format!("session:resume_preflight: {e}"))?;

                let bound_config_dir = crate::identity::resolver::resolve_bound_oauth_config_dir(
                    &mstore,
                    &id_store,
                    &identity_store,
                    &cmd.block_id,
                );
                let mut input = preflight_input_from_meta(&block.meta, bound_config_dir);
                let session_id_field = obj::meta_get_string(&block.meta, "agent:session_id_field", "session_id");
                let block_id = cmd.block_id.clone();

                // Blocking I/O (a transcript tail read, one `is_file`, at most
                // one `read_dir` of a single directory) off the async runtime's
                // worker threads — small, but a pane open shouldn't be able to
                // stall the reactor on a cold or network-backed home directory.
                let result = tokio::task::spawn_blocking(move || {
                    if input.session_id.is_empty() {
                        input.history_session_id =
                            crate::backend::blockcontroller::persistent::pane_history_session_id(
                                Some(&filestore),
                                Some(&mstore),
                                &block_id,
                                &session_id_field,
                            )
                            .unwrap_or_default();
                    }
                    // The head of the agent's chain, for the spawn's resume
                    // gate. The UID comes from the pane's agent row, as the
                    // spawn env's does (`persisted_agent_identity`).
                    input.chain_head = mstore
                        .instance_get_active_for_block(&block_id)
                        .ok()
                        .flatten()
                        .map(|instance| instance.id)
                        .filter(|uid| !uid.trim().is_empty())
                        .and_then(|uid| {
                            let gfs = crate::backend::agent_session::global_transcript_store()?;
                            crate::backend::continuity_segments::chain_head(gfs, uid.trim())
                                .map(crate::backend::continuity_segments::with_legacy_identity)
                        });
                    input.identity_key =
                        crate::identity::account_email::identity_key_from_oauth_dir("claude", &input.config_dir);
                    crate::backend::resume_preflight::preflight(&input)
                })
                .await
                .map_err(|e| format!("session:resume_preflight: {e}"))?;

                tracing::info!(
                    block_id = %cmd.block_id,
                    verdict = %result.verdict.as_str(),
                    duration_ms = result.duration_ms,
                    "session:resume_preflight"
                );

                Ok(SessionResumePreflightResult {
                    block_id: cmd.block_id,
                    verdict: result.verdict.as_str().to_string(),
                    session_id: result.session_id,
                    recoverable_session_id: result.recoverable_session_id,
                    steps: result
                        .steps
                        .into_iter()
                        .map(|s| ResumePreflightStep {
                            id: s.id.to_string(),
                            label: s.label,
                            ok: s.ok,
                            detail: s.detail,
                            duration_ms: s.duration_ms,
                        })
                        .collect(),
                    duration_ms: result.duration_ms,
                })
            }
        },
    );
}

/// Lift a block's meta into a [`resume_preflight::PreflightInput`].
///
/// **Every default here must match what the real spawn path uses for the
/// same key**, or the preflight predicts something the spawn won't do —
/// which is worse than not predicting at all, since the pane then states a
/// falsehood confidently.
///
/// `agent:resume_flag` defaulting to `"--resume"` is that rule doing real
/// work (#2833): it was `""` here while all five real spawn-path readers
/// default to `"--resume"` (the persistent and subprocess branches of
/// `agent_handlers/input.rs`'s `run_agent_turn` and of
/// `app_api/agent_io.rs`'s `register_agent_send`, plus the eager-resume path
/// in `eager_resume.rs` — the persistent branch of `run_agent_turn` is the
/// one that builds `PersistentSpawnConfig` for a pane's message send).
/// `agent_open.rs` only started writing the key recently, so any Claude pane
/// created before that and not respawned since has no `agent:resume_flag`
/// at all: the spawn still attaches `--resume`, but the preflight was
/// reporting `Unknown` and silently suppressing the notice for exactly the
/// long-lived panes this feature exists for.
/// `bound_config_dir` is `identity::resolver::resolve_bound_oauth_config_dir`'s
/// answer for this block, and **takes precedence over `cmd:env`** whenever
/// it's `Some` (reagent P1, second pass on PR #2833).
///
/// For an agent bound to an Armory OAuth identity, the `cmd:env` snapshot is
/// simply not where the CLI will look: the real spawn resolves the isolated
/// dir dynamically through `inject_identity_env_async` (via
/// `agent_handlers/input.rs`'s `build_persistent_spawn_env`, which
/// `app_api/agent_io.rs`'s `agent_send_spawn_env` also calls), and
/// `reactive.rs` already documents that "identity-bound agents' real
/// `CLAUDE_CONFIG_DIR` is never the stale `cmd:env` snapshot"
/// (`SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md`).
/// Checking reachability against the wrong directory is worse than not
/// checking: it can warn "fresh" at a pane that will resume perfectly well,
/// or stay quiet at one that's about to lose its conversation.
///
/// `None` — not identity-bound, or a non-OAuth provider — keeps the
/// `cmd:env` read, which is correct for those. Same helper and same
/// precedence order `subagent_watcher` already uses for this exact
/// pre-spawn question.
fn preflight_input_from_meta(
    meta: &crate::backend::obj::MetaMapType,
    bound_config_dir: Option<std::path::PathBuf>,
) -> crate::backend::resume_preflight::PreflightInput {
    // `CLAUDE_CONFIG_DIR` from the block's own `cmd:env` map — the fallback
    // when this pane isn't identity-bound.
    let config_dir = match bound_config_dir {
        Some(dir) => dir.to_string_lossy().to_string(),
        None => crate::backend::blockcontroller::cmd_env_of(meta)
            .remove("CLAUDE_CONFIG_DIR")
            .unwrap_or_default(),
    };

    crate::backend::resume_preflight::PreflightInput {
        resume_flag: obj::meta_get_string(meta, "agent:resume_flag", "--resume"),
        session_id: obj::meta_get_string(meta, "agent:sessionid", ""),
        working_dir: obj::meta_get_string(meta, "cmd:cwd", ""),
        config_dir,
        history_session_id: String::new(),
        chain_head: None,
        identity_key: None,
    }
}

fn register_session_archive_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let filestore = state.filestore.clone();
    let broker = state.broker.clone();

    engine.register_typed(
        COMMAND_SESSION_ARCHIVE,
        move |cmd: CommandSessionArchiveData, _ctx| {
            let mstore = mstore.clone();
            let filestore = filestore.clone();
            let broker = broker.clone();
            async move {

                tracing::info!(block_id = %cmd.block_id, "session:archive");

                let archive_dir = session_archive::default_archive_dir()
                    .ok_or_else(|| "cannot determine home directory".to_string())?;

                // The delete and its announcement under the block's
                // transcript order lock, so no in-flight append's write and
                // event can interleave with them (review of #3636).
                let (archived_bytes, archived_at) =
                    crate::backend::blockcontroller::shell::with_transcript_order(&cmd.block_id, || {
                        let archived = session_archive::archive_session_output(
                            &mstore,
                            &filestore,
                            &cmd.block_id,
                            &archive_dir,
                        )?;
                        // The block's transcript is gone: open panes resync now.
                        crate::backend::blockcontroller::shell::publish_transcript_changed(
                            &broker,
                            &cmd.block_id,
                            crate::backend::mps::FILE_OP_DELETE,
                            &filestore,
                        );
                        Ok::<_, String>(archived)
                    })?;

                Ok(SessionArchiveResult {
                block_id: cmd.block_id,
                archived_bytes,
                archived_at,
            })
            }
        },
    );
}

fn register_session_restore_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let filestore = state.filestore.clone();
    let broker = state.broker.clone();

    engine.register_typed(
        COMMAND_SESSION_RESTORE,
        move |cmd: CommandSessionRestoreData, _ctx| {
            let mstore = mstore.clone();
            let filestore = filestore.clone();
            let broker = broker.clone();
            async move {

                tracing::info!(block_id = %cmd.block_id, "session:restore");

                // The replace and its announcement under the block's
                // transcript order lock (review of #3636): no append's write
                // and event can land between them, so a pane sees every
                // event of the old generation, then the replace, then the
                // new generation's.
                let restored_bytes =
                    crate::backend::blockcontroller::shell::with_transcript_order(&cmd.block_id, || {
                        let restored = session_archive::restore_session_output(
                            &mstore,
                            &filestore,
                            &cmd.block_id,
                        )?;
                        // Replaced content, a new generation: open panes resync now.
                        crate::backend::blockcontroller::shell::publish_transcript_changed(
                            &broker,
                            &cmd.block_id,
                            crate::backend::mps::FILE_OP_REPLACE,
                            &filestore,
                        );
                        Ok::<_, String>(restored)
                    })?;

                Ok(SessionRestoreResult {
                block_id: cmd.block_id,
                restored_bytes,
            })
            }
        },
    );
}

fn register_session_export_handler(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let filestore = state.filestore.clone();

    engine.register_typed(
        COMMAND_SESSION_EXPORT,
        move |cmd: CommandSessionExportData, _ctx| {
            let mstore = mstore.clone();
            let filestore = filestore.clone();
            async move {

                tracing::info!(block_id = %cmd.block_id, "session:export");

                let (raw_bytes, line_count) = session_archive::read_session_output(
                    &mstore,
                    &filestore,
                    &cmd.block_id,
                )?;

                let byte_count = raw_bytes.len() as u64;
                let content = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);

                Ok(SessionExportResult {
                content,
                line_count,
                byte_count,
            })
            }
        },
    );
}

fn empty_summary_result() -> ActivitySummaryResult {
    ActivitySummaryResult { summary: String::new(), tokens: None }
}

fn register_session_activity_summary(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let filestore = state.filestore.clone();
    let event_bus = state.event_bus.clone();

    engine.register_typed(
        COMMAND_SESSION_ACTIVITY_SUMMARY,
        move |cmd: CommandActivitySummaryData, _ctx| {
            let mstore = mstore.clone();
            let filestore = filestore.clone();
            let event_bus = event_bus.clone();
            async move {
                // Admit through the Ambient Model Call gateway BEFORE doing any
                // work: a stale (superseded) request does zero FileStore reads
                // or prompt building, not just skips the CLI spawn. The pull
                // semaphore caps concurrent Haiku spawns across all blocks, raced
                // against cancellation so a request superseded while queued for
                // a permit never spawns the CLI at all. See
                // docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md.
                let Some(slot) = ambient::call::admit(
                    &ambient::purpose::ACTIVITY_SUMMARY,
                    cmd.block_id.clone(),
                    cmd.generation,
                )
                .await
                else {
                    return Ok(empty_summary_result());
                };

                let word_target = cmd.word_target.unwrap_or(7).max(3).min(20);

                let block: Block = mstore
                    .get(&cmd.block_id)
                    .map_err(|e| format!("session:activity_summary: {e}"))?
                    .ok_or_else(|| format!("BLOCK_NOT_FOUND: {}", cmd.block_id))?;

                // The user's newest message, verbatim — the frontend passes this
                // directly from the just-submitted TurnStart content, so the
                // common case needs no FileStore read at all. Without one, the
                // session's recent activity is read instead, and shown to the
                // model as activity: it used to be passed off as "The user just
                // said". See
                // docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md.
                let user_message = cmd
                    .user_message
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let activity = match user_message {
                    Some(_) => None,
                    None => ambient::digest::read_recent_activity_digest(&filestore, &cmd.block_id),
                };

                // A stored value that is not a real title (a placeholder an older build
                // accepted, such as `(none yet)`) counts as NO title: it is never fed
                // back into the prompt, so it cannot sustain itself. The first draft
                // fed it back as the "current title" and told the model to repeat it.
                let stored_title = obj::meta_get_string(&block.meta, ambient::title::META_TITLE, "");
                let current_title = if ambient::validate::is_usable_title(&stored_title) {
                    stored_title
                } else {
                    String::new()
                };

                // Nothing to anchor a title on AND nothing new to evaluate —
                // matches the old digest-empty early return.
                if user_message.is_none() && activity.is_none() && current_title.is_empty() {
                    slot.abandon(ambient::outcome::Outcome::EmptyDigest);
                    return Ok(empty_summary_result());
                }

                let Some(target) = ambient::call::CliTarget::from_meta(&block.meta) else {
                    tracing::debug!(block_id = %cmd.block_id, "session:activity_summary: no CLI path in meta");
                    return Ok(empty_summary_result());
                };

                let prompt = ambient::prompt::build_session_title_prompt(
                    &current_title,
                    user_message.as_deref(),
                    activity.as_deref(),
                    word_target,
                );
                let limits = ambient::validate::title_limits(word_target);
                let reply = slot
                    .run(&target, &prompt, |raw| {
                        ambient::reply::judge_line(raw, |t| ambient::validate::accept_line(t, &limits))
                    })
                    .await;

                // Stored here, not by the pane: one writer with the recovery sweep,
                // in one transaction, so a rewording never replaces the title and
                // neither writer overwrites what the other stored while its call ran
                // (`ambient::title`). A call superseded by a newer message was
                // cancelled by the gateway and has no text.
                let mut stored = false;
                if !reply.text.is_empty() {
                    match ambient::title::store_title(&mstore, &cmd.block_id, &reply.text, ambient::title::Replace::IfNews) {
                        Ok(true) => {
                            stored = true;
                            crate::backend::blockcontroller::core::broadcast_block_update(&mstore, &event_bus, &cmd.block_id);
                        }
                        Ok(false) => {}
                        Err(e) => tracing::warn!(block_id = %cmd.block_id, error = %e, "session:activity_summary: could not store the title"),
                    }
                }
                Ok(ActivitySummaryResult { summary: if stored { reply.text } else { String::new() }, tokens: reply.tokens })
            }
        },
    );
}

fn empty_suggestion_result() -> NextPromptSuggestionResult {
    NextPromptSuggestionResult { suggestion: String::new(), tokens: None }
}

fn register_session_next_prompt_suggestion(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    let filestore = state.filestore.clone();

    engine.register_typed(
        COMMAND_SESSION_NEXT_PROMPT_SUGGESTION,
        move |cmd: CommandNextPromptSuggestionData, _ctx| {
            let mstore = mstore.clone();
            let filestore = filestore.clone();
            async move {
                // Same admission discipline and pull-call cap as activity_summary.
                // Ghost text has a sharper failure mode than the read-only summary
                // (a stale suggestion can put words in the user's mouth), so
                // admitting before any work matters just as much here.
                let Some(slot) = ambient::call::admit(
                    &ambient::purpose::NEXT_PROMPT_SUGGESTION,
                    cmd.block_id.clone(),
                    cmd.generation,
                )
                .await
                else {
                    return Ok(empty_suggestion_result());
                };

                let block: Block = mstore
                    .get(&cmd.block_id)
                    .map_err(|e| format!("session:next_prompt_suggestion: {e}"))?
                    .ok_or_else(|| format!("BLOCK_NOT_FOUND: {}", cmd.block_id))?;

                // The pane's own translated conversation when it sent one (any
                // provider); otherwise the output file, which only Claude-shaped
                // streams can be read from.
                let activity = match cmd.activity.filter(|entries| !entries.is_empty()) {
                    Some(entries) => ambient::digest::activity_from_entries(&cmd.block_id, entries),
                    None => ambient::digest::read_recent_activity(&filestore, &cmd.block_id),
                };
                let Some(activity) = activity else {
                    slot.abandon(ambient::outcome::Outcome::EmptyDigest);
                    return Ok(empty_suggestion_result());
                };
                // The turn ended waiting for the user (a question, a tool that asks
                // them) or before the assistant answered: there is no next
                // instruction to predict, so no call. Asked to judge this itself,
                // the model wrote prose about declining, and it reached the composer.
                if activity.ending != ambient::digest::TurnEnding::Statement {
                    slot.abandon(ambient::outcome::Outcome::Gated);
                    return Ok(empty_suggestion_result());
                }

                let Some(target) = ambient::call::CliTarget::from_meta(&block.meta) else {
                    tracing::debug!(block_id = %cmd.block_id, "session:next_prompt_suggestion: no CLI path in meta");
                    return Ok(empty_suggestion_result());
                };

                let prompt = ambient::prompt::build_next_prompt_prompt(&activity.text);
                let reply = slot
                    .run(&target, &prompt, |raw| ambient::reply::judge_line(raw, ambient::validate::accept_next_prompt))
                    .await;

                // The tokens were spent either way, so they are still reported;
                // only the text is withheld when it is not a usable next prompt.
                Ok(NextPromptSuggestionResult { suggestion: reply.text, tokens: reply.tokens })
            }
        },
    );
}

#[cfg(test)]
mod preflight_input_tests {
    use super::preflight_input_from_meta;

    // ── resume-preflight meta extraction (reagent P1 on PR #2833) ────────
    //
    // The divergence these guard: the preflight must read every meta key with
    // the SAME default the real spawn path uses, or it predicts a spawn that
    // won't happen. Unit-testing `preflight` itself (as the rest of the suite
    // does) can't catch that — it takes an explicit `PreflightInput`, so the
    // extraction defaults are exactly the part those tests skip.

    fn meta_of(pairs: &[(&str, serde_json::Value)]) -> crate::backend::obj::MetaMapType {
        let mut m = crate::backend::obj::MetaMapType::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        m
    }

    /// The regression itself: a Claude pane predating `agent_open.rs` writing
    /// `agent:resume_flag`. The spawn attaches `--resume` via its own default,
    /// so the preflight must not read this as "provider can't resume" and go
    /// silent.
    #[test]
    fn absent_resume_flag_defaults_to_the_same_value_the_spawn_path_uses() {
        let input = preflight_input_from_meta(&meta_of(&[]), None);
        assert_eq!(
            input.resume_flag, "--resume",
            "must match agent_handlers/input.rs:400's default, or the preflight              reports Unknown for panes whose spawn really will --resume",
        );
    }

    /// A provider that genuinely has no resume flag writes an explicit empty
    /// string; that must survive as empty rather than being back-filled with
    /// the default, or the preflight would claim resume support that isn't there.
    #[test]
    fn an_explicitly_empty_resume_flag_is_preserved_not_defaulted() {
        let input = preflight_input_from_meta(&meta_of(&[("agent:resume_flag", serde_json::json!(""))]), None);
        assert_eq!(input.resume_flag, "");
    }

    #[test]
    fn an_explicit_resume_flag_is_read_verbatim() {
        let input = preflight_input_from_meta(&meta_of(&[("agent:resume_flag", serde_json::json!("-r"))]), None);
        assert_eq!(input.resume_flag, "-r");
    }

    #[test]
    fn config_dir_is_read_out_of_the_cmd_env_map() {
        let input = preflight_input_from_meta(&meta_of(&[(
            "cmd:env",
            serde_json::json!({ "CLAUDE_CONFIG_DIR": "/home/dev/.claude", "OTHER": "x" }),
        )]), None);
        assert_eq!(input.config_dir, "/home/dev/.claude");
    }

    /// No `cmd:env` at all, or no `CLAUDE_CONFIG_DIR` within it, must yield an
    /// empty config dir — `preflight` turns that into `Unknown` and stays
    /// silent, which is the correct posture when there's nowhere to look.
    #[test]
    fn a_missing_config_dir_is_empty_rather_than_a_guess() {
        assert_eq!(preflight_input_from_meta(&meta_of(&[]), None).config_dir, "");
        assert_eq!(
            preflight_input_from_meta(&meta_of(&[("cmd:env", serde_json::json!({ "PATH": "/usr/bin" }))]), None)
                .config_dir,
            ""
        );
        assert_eq!(
            preflight_input_from_meta(&meta_of(&[("cmd:env", serde_json::json!("not-an-object"))]), None).config_dir,
            ""
        );
    }

    #[test]
    fn session_id_and_working_dir_default_to_empty() {
        let input = preflight_input_from_meta(&meta_of(&[]), None);
        assert_eq!(input.session_id, "");
        assert_eq!(input.working_dir, "");

        let input = preflight_input_from_meta(&meta_of(&[
            ("agent:sessionid", serde_json::json!("sid-1")),
            ("cmd:cwd", serde_json::json!("/work/dir")),
        ]), None);
        assert_eq!(input.session_id, "sid-1");
        assert_eq!(input.working_dir, "/work/dir");
    }

    /// reagent P1 (second pass): an identity-bound pane's real config dir
    /// comes from the identity resolver, never the `cmd:env` snapshot, so a
    /// resolved dir must win outright — including when `cmd:env` disagrees.
    #[test]
    fn a_bound_identity_config_dir_overrides_the_cmd_env_snapshot() {
        let meta = meta_of(&[(
            "cmd:env",
            serde_json::json!({ "CLAUDE_CONFIG_DIR": "/stale/from/spawn/snapshot" }),
        )]);
        let input = preflight_input_from_meta(
            &meta,
            Some(std::path::PathBuf::from("/identities/acct-7/claude")),
        );
        assert_eq!(
            input.config_dir, "/identities/acct-7/claude",
            "the identity-resolved dir is where the CLI will actually look",
        );
    }

    /// …and a resolved dir must still win when there's no `cmd:env` at all.
    #[test]
    fn a_bound_identity_config_dir_is_used_with_no_cmd_env_present() {
        let input = preflight_input_from_meta(
            &meta_of(&[]),
            Some(std::path::PathBuf::from("/identities/acct-9/claude")),
        );
        assert_eq!(input.config_dir, "/identities/acct-9/claude");
    }

    /// `None` means "not identity-bound, or not an OAuth provider" — for
    /// those the `cmd:env` snapshot IS what the spawn uses, so it must still
    /// be read rather than dropped.
    #[test]
    fn an_unbound_pane_still_falls_back_to_cmd_env() {
        let meta = meta_of(&[("cmd:env", serde_json::json!({ "CLAUDE_CONFIG_DIR": "/home/dev/.claude" }))]);
        assert_eq!(preflight_input_from_meta(&meta, None).config_dir, "/home/dev/.claude");
    }
}
