// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Backend half of the `/btw` slash command — a faithful recreation of
//! Claude Code CLI's own native `/btw`: a fast, one-shot, tool-less answer
//! that is aware of the current conversation (via a text prefix, not
//! `--resume`), WITHOUT touching the asking pane's persisted transcript and
//! WITHOUT blocking/queuing behind that pane's own live turn.
//!
//! ## Why a throwaway block
//!
//! `SubprocessController::spawn_turn`
//! (`blockcontroller/subprocess/host_spawn.rs`) is strictly one-turn-at-a-
//! time PER BLOCK — a second `spawn_turn` on a block that's already running
//! queues behind it (`inner.pending_messages`), plus there's a cross-
//! process session-ownership lease on top. Calling it again on the SAME
//! block id that owns the pane's live turn would therefore queue behind
//! that turn and defeat the entire point of `/btw`.
//!
//! So this spawns the one-shot turn against a FRESH, throwaway block id —
//! a different block id means a different `SubprocessController` instance
//! (`blockcontroller`'s registry is keyed per block), so there is no
//! queue/lease contention with the asking pane's real live turn, while
//! still going through the exact same `run_agent_turn`/`AgentTurnDeps`
//! pipeline every real agent turn uses — identity injection, the OAuth
//! spawn gate that "fails closed", muxbus/bashwrap env, PATH, git identity
//! — instead of hand-rolling a second, parallel spawn path that bypasses
//! all of it. See `input.rs`'s `run_agent_turn` doc comment for why that
//! machinery must not be reimplemented.
//!
//! The throwaway block deliberately carries no `agentId` / `agentName` /
//! `agentMode` / `agent:*` meta:
//!   - No `agentId` ⇒ `shell::resolve_global_output_zone` (file_ops.rs)
//!     resolves to `None` for this block, so `spawn_turn`'s own stdout
//!     mirroring never writes into any real agent's persisted
//!     `agent:<defId>:current` transcript zone.
//!   - It is never linked into any tab/layout tree and is deleted (with
//!     its controller and MPS-persisted history) the moment the turn ends
//!     — see `cleanup_throwaway_block`.
//!   - Known scope limitation: with no `db_agent_instances` row bound to
//!     this block, `inject_identity_env_async`
//!     (`identity/resolver/inject.rs`) takes its documented "block has no
//!     agent instance row — nothing to inject" early return, the same
//!     behavior an ad hoc quick-launch pane already gets. A `/btw` asked
//!     from a pane whose agent is bound to a specific non-ambient identity
//!     does not inherit that binding; it runs on ambient credentials.
//!     Reproducing the source pane's exact bound-identity resolution would
//!     mean minting (and tearing down) a real `db_agent_instances` row for
//!     a block that is never a real agent — left as a follow-up.
//!   - Similarly, no `agentMode` meta ⇒ always the plain host subprocess
//!     branch, never the container `docker exec` branch, regardless of
//!     what the source pane is. A `/btw` from a container agent's pane
//!     therefore answers from the HOST's view of the provider CLI.
//!
//! ## Providers
//!
//! `/btw` is an AgentMux feature, not a pass-through to a CLI's own, so it
//! runs on Claude, Codex and Gemini panes alike ([`BtwProvider`], from the
//! source pane's `agentProvider`). Each gets its own tool-less one-shot
//! argv — `build_side_question_argv`, `build_codex_side_question_argv`,
//! `build_gemini_side_question_argv` — and its own answer translator. The
//! turn being tool-less on every provider is what keeps a side question
//! outside the agent identity system: it can't call the agentmux tools,
//! so it never acts as the pane's agent (#3579). Other providers are
//! refused with an error chunk.
//!
//! ## Output contract
//!
//! The turn's own stdout is written to the THROWAWAY block's own
//! FileStore-backed `output` file by `spawn_turn` exactly like any other
//! turn (needed so this module can read it back) — but that file's block
//! scope (`block:<throwaway_id>`) is never surfaced to any pane; nothing
//! is ever subscribed to it, and the block + its persisted history are
//! deleted as soon as the turn ends.
//!
//! Instead, this module polls that file for newly-appended JSON lines
//! (there is no in-process "call me back on new bytes" primitive in this
//! codebase — `reactive::progress_watcher` polls the same way for the same
//! reason), translates each line through the provider's translator
//! (`agents/translator/` — for Claude, the SAME `ClaudeTranslator` the
//! drone Agent block runner uses), and republishes one `mps::EVENT_BTW_ANSWER_CHUNK` per translated
//! `AgentEvent` — scoped to the SOURCE pane's block id (`block_id` in the
//! request) plus the minted `request_id`, never the throwaway block's own
//! scope. A final chunk carries `"done": true`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::agents::translator::claude::ClaudeTranslator;
use crate::agents::translator::codex::CodexTranslator;
use crate::agents::translator::gemini::GeminiTranslator;
use crate::agents::translator::Translator;
use crate::agents::types::AgentEvent;
use crate::backend::blockcontroller;
use crate::backend::blockcontroller::subprocess::argv::{
    build_codex_side_question_argv, build_gemini_side_question_argv, build_side_question_argv,
    model_flag_value, GEMINI_DENY_ALL_TOOLS_POLICY,
};
use crate::backend::mps;
use crate::backend::obj::{Block, MetaMapType};
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    AskSideQuestionResult, CommandAskSideQuestionData, COMMAND_ASK_SIDE_QUESTION,
};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

use super::super::AppState;
use super::input::{run_agent_turn, AgentTurnDeps, TurnRegistration};

/// Meta keys deliberately copied from the source block onto the throwaway
/// block — its whole one-shot CLI configuration, nothing agent-identity-
/// shaped. See this module's doc comment for what's excluded and why.
const COPIED_META_KEYS: &[&str] = &["cmd", "cmd:cwd", "cmd:env"];

/// Max time to wait for a translated terminal event (`AgentEvent::Done` /
/// `Error`) before giving up and synthesizing one. A `--max-turns 1`
/// tool-less turn should finish in a few seconds; this is a generous
/// backstop against a wedged/never-exiting subprocess, not the expected
/// path.
const MAX_WAIT: Duration = Duration::from_secs(120);
/// How often to re-check the throwaway block's output file for new bytes.
/// Tight relative to `reactive::progress_watcher`'s 3s sweep — that loop
/// amortizes a fixed per-tick cost across every live agent block; this one
/// drives a single, short-lived, tool-less turn where a snappy overlay
/// matters more than sweep cost.
const POLL_INTERVAL: Duration = Duration::from_millis(120);
/// Cap on bytes read per poll — defensive only; a tool-less, single-turn
/// response is never remotely this large.
const MAX_READ_BYTES: i64 = 1024 * 1024;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let deps = AgentTurnDeps::from_state(state);
    let mstore = state.mstore.clone();
    let filestore = state.filestore.clone();
    let broker = state.broker.clone();
    let boot_id = state.boot_id.clone();

    engine.register_typed(
        COMMAND_ASK_SIDE_QUESTION,
        move |cmd: CommandAskSideQuestionData, _ctx| {
            let deps = deps.clone();
            let mstore = mstore.clone();
            let filestore = filestore.clone();
            let broker = broker.clone();
            let boot_id = boot_id.clone();
            async move {
                let request_id = uuid::Uuid::new_v4().to_string();
                tracing::info!(
                    source_block_id = %cmd.block_id,
                    request_id = %request_id,
                    "AskSideQuestion (/btw)"
                );

                // The RPC itself must not block on (or queue behind) any
                // turn — it mints request_id and returns immediately so
                // the frontend can start listening for
                // EVENT_BTW_ANSWER_CHUNK before any chunk could possibly
                // have been published yet. The real turn runs entirely in
                // this detached task.
                tokio::spawn(run_side_question(
                    deps,
                    mstore,
                    filestore,
                    broker,
                    boot_id,
                    cmd.block_id,
                    cmd.question,
                    cmd.context_snapshot,
                    request_id.clone(),
                ));

                Ok(AskSideQuestionResult { request_id })
            }
        },
    );
}

/// The MPS scope a `/btw` answer streams on — the SOURCE pane's block id,
/// never the throwaway block that actually ran the turn.
fn btw_scope(source_block_id: &str, request_id: &str) -> String {
    format!("block:{source_block_id}:btw:{request_id}")
}

/// The prompt sent to the CLI: `context_snapshot` prepended to `question`.
/// No `--resume` on this path — live session context is deliberately
/// carried as a text prefix instead (concurrent `--resume` on a live
/// session is unsafe per this module's own doc comment).
fn build_prompt(context_snapshot: &str, question: &str) -> String {
    if context_snapshot.trim().is_empty() {
        question.to_string()
    } else {
        format!("{context_snapshot}\n\n{question}")
    }
}

/// The CLI a `/btw` runs on — the source pane's. See the module doc's
/// "Providers" section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BtwProvider {
    Claude,
    Codex,
    Gemini,
}

impl BtwProvider {
    /// From the source pane's `agentProvider`. A pane without one is a
    /// Claude pane — the only kind `/btw` served before the others.
    fn of(source_meta: &MetaMapType) -> Result<Self, String> {
        match crate::backend::obj::meta_get_string(source_meta, "agentProvider", "").as_str() {
            "" | "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "gemini" => Ok(Self::Gemini),
            other => Err(format!("/btw isn't available for {other} agents")),
        }
    }

    fn cli_command(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
        }
    }

    fn translator(self) -> Box<dyn Translator> {
        match self {
            Self::Claude => Box::new(ClaudeTranslator::new()),
            Self::Codex => Box::new(CodexTranslator::new()),
            Self::Gemini => Box::new(GeminiTranslator::new()),
        }
    }
}

/// Write [`GEMINI_DENY_ALL_TOOLS_POLICY`] where `--policy` can read it, and
/// return its path. Under the AgentMux data dir rather than a shared temp
/// dir, where another local user could swap in a permissive policy.
/// Written to a temp file and renamed, so a concurrent `/btw` never reads
/// a half-written policy.
fn ensure_gemini_policy() -> Result<String, String> {
    ensure_gemini_policy_in(&crate::backend::base::get_mux_data_dir().join("btw"))
}

fn ensure_gemini_policy_in(dir: &std::path::Path) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("btw: create {}: {e}", dir.display()))?;
    let path = dir.join("gemini-deny-all-tools.toml");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(GEMINI_DENY_ALL_TOOLS_POLICY) {
        let tmp = dir.join(format!(".gemini-deny-all-tools.{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, GEMINI_DENY_ALL_TOOLS_POLICY)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("btw: write {}: {e}", path.display())
            })?;
    }
    Ok(path.to_string_lossy().into_owned())
}

/// Build the throwaway block's meta from the source block's — its one-shot
/// CLI config with `provider`'s tool-less `/btw` argv, minus every
/// agent-identity-shaped key. `gemini_policy` is the deny-all policy path
/// (read only for Gemini). Pure and unit-testable; see the module doc
/// comment for the exclusion rationale.
fn build_throwaway_meta(source_meta: &MetaMapType, provider: BtwProvider, gemini_policy: &str) -> MetaMapType {
    let mut meta = MetaMapType::new();
    for key in COPIED_META_KEYS {
        if let Some(v) = source_meta.get(*key) {
            meta.insert((*key).to_string(), v.clone());
        }
    }
    if meta.get("cmd").is_none() {
        meta.insert("cmd".to_string(), Value::String(provider.cli_command().to_string()));
    }

    let source_args: Option<Vec<String>> = match source_meta.get("cmd:args") {
        Some(Value::Array(arr)) => Some(
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
        ),
        _ => None,
    };
    let model = source_args.as_deref().and_then(model_flag_value);
    let cli_args = match provider {
        BtwProvider::Claude => build_side_question_argv(&source_args.unwrap_or_else(|| {
            vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
            ]
        })),
        BtwProvider::Codex => build_codex_side_question_argv(model.as_deref()),
        BtwProvider::Gemini => build_gemini_side_question_argv(gemini_policy, model.as_deref()),
    };
    meta.insert(
        "cmd:args".to_string(),
        Value::Array(cli_args.into_iter().map(Value::String).collect()),
    );

    meta
}

/// Create the throwaway block and drive it through `run_agent_turn`. Returns
/// the throwaway block id on success — including when `run_agent_turn`
/// itself fails after the block was created (the block + controller still
/// exist and get cleaned up normally; `drain_and_publish`'s own
/// not-running-with-nothing-published fallback surfaces that failure, and
/// in the common failure case — the identity/OAuth spawn gate — the block's
/// own output file already carries a translatable error `result` frame, so
/// the real message reaches the frontend either way). Only returns `Err`
/// when nothing was ever created (source block missing, insert failed) —
/// there is then genuinely nothing to poll or clean up.
async fn spawn_throwaway_turn(
    deps: &AgentTurnDeps,
    mstore: &Arc<Store>,
    boot_id: &Arc<str>,
    source_block_id: &str,
    prompt: String,
) -> Result<(String, BtwProvider), String> {
    let source: Block = mstore
        .get(source_block_id)
        .map_err(|e| format!("btw: load source block: {e}"))?
        .ok_or_else(|| format!("btw: source block {source_block_id} not found"))?;

    let provider = BtwProvider::of(&source.meta)?;
    let gemini_policy = match provider {
        BtwProvider::Gemini => ensure_gemini_policy()?,
        _ => String::new(),
    };
    let meta = build_throwaway_meta(&source.meta, provider, &gemini_policy);

    let block_id = uuid::Uuid::new_v4().to_string();
    let mut block = Block {
        oid: block_id.clone(),
        meta,
        ..Default::default()
    };
    mstore
        .insert(&mut block)
        .map_err(|e| format!("btw: insert throwaway block: {e}"))?;

    let ctrl = blockcontroller::subprocess::SubprocessController::new(
        String::new(), // tab_id — never tab-attached
        block_id.clone(),
        Some(deps.broker.clone()),
        // event_bus: no obj:update broadcasts needed for a block that
        // never appears in any layout/tab tree.
        None,
        Some(mstore.clone()),
        Some(deps.filestore_gate.clone()),
        mstore.shared_agent_registry(),
        boot_id.clone(),
    );
    let ctrl = Arc::new(ctrl);
    ctrl.set_self_ref();
    blockcontroller::register_controller(&block_id, ctrl);

    if let Err(e) = run_agent_turn(
        deps,
        block_id.clone(),
        prompt,
        None,
        TurnRegistration::Skip,
        crate::backend::blockcontroller::health::TurnOrigin::System,
    )
    .await
    {
        tracing::warn!(
            block_id = %block_id,
            error = %e,
            "btw: one-shot turn failed to start"
        );
        // Fall through — see this function's own doc comment.
    }

    Ok((block_id, provider))
}

/// Drive one `/btw` request end to end: spawn the throwaway turn, drain +
/// translate + republish its output, then clean up.
async fn run_side_question(
    deps: AgentTurnDeps,
    mstore: Arc<Store>,
    filestore: Arc<FileStore>,
    broker: Arc<mps::Broker>,
    boot_id: Arc<str>,
    source_block_id: String,
    question: String,
    context_snapshot: String,
    request_id: String,
) {
    let scope = btw_scope(&source_block_id, &request_id);
    let prompt = build_prompt(&context_snapshot, &question);

    let (throwaway_block_id, provider) = match spawn_throwaway_turn(
        &deps,
        &mstore,
        &boot_id,
        &source_block_id,
        prompt,
    )
    .await
    {
        Ok(spawned) => spawned,
        Err(e) => {
            publish_chunk(
                &broker,
                &scope,
                &source_block_id,
                &request_id,
                &AgentEvent::Error { message: e },
                true,
            );
            return;
        }
    };

    drain_and_publish(
        &broker,
        &filestore,
        provider.translator(),
        &scope,
        &source_block_id,
        &request_id,
        &throwaway_block_id,
    )
    .await;

    cleanup_throwaway_block(&mstore, &broker, &throwaway_block_id);
}

/// Kill the throwaway controller, delete its block row, and purge any MPS-
/// persisted history under its scope — mirrors `COMMAND_DELETE_SUB_BLOCK`'s
/// kill-then-delete ordering (`server/websocket.rs`). Best-effort: this
/// runs after the turn is already over, so a failure here is a leftover
/// row/controller to log, not a failed `/btw` answer.
fn cleanup_throwaway_block(mstore: &Arc<Store>, broker: &mps::Broker, throwaway_block_id: &str) {
    blockcontroller::delete_controller(throwaway_block_id);
    if let Err(e) = mstore.delete::<Block>(throwaway_block_id) {
        tracing::warn!(
            block_id = %throwaway_block_id,
            error = %e,
            "btw: failed to delete throwaway block row"
        );
    }
    broker.purge_scope(&format!("block:{throwaway_block_id}"));
}

/// Poll the throwaway block's own `output` FileStore subject for newly
/// appended stream-json lines, translate each into `AgentEvent`s, and
/// publish one `EVENT_BTW_ANSWER_CHUNK` per event on `scope`. Returns once
/// a terminal event (`Done`/`Error`) has been published, or after
/// synthesizing one on timeout / unexpected exit. See the module doc
/// comment for why this polls FileStore directly instead of hooking into
/// `spawn_turn`'s own output path.
async fn drain_and_publish(
    broker: &mps::Broker,
    filestore: &FileStore,
    mut translator: Box<dyn Translator>,
    scope: &str,
    source_block_id: &str,
    request_id: &str,
    throwaway_block_id: &str,
) {
    let mut offset: i64 = 0;
    let deadline = tokio::time::Instant::now() + MAX_WAIT;

    loop {
        let mut saw_terminal = false;

        if let Ok(Some(info)) = filestore.stat(throwaway_block_id, "output") {
            let size = info.size;
            if size > offset {
                let want = (size - offset).min(MAX_READ_BYTES);
                if let Ok((_, bytes)) = filestore.read_at(throwaway_block_id, "output", offset, want) {
                    if !bytes.is_empty() {
                        // Only consume up to the last complete line — a
                        // read boundary can land mid-line.
                        let consumed = match bytes.iter().rposition(|b| *b == b'\n') {
                            Some(idx) => idx + 1,
                            None => bytes.len(),
                        };
                        offset += consumed as i64;
                        let text = String::from_utf8_lossy(&bytes[..consumed]);
                        for line in text.lines() {
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }
                            let Ok(frame) = serde_json::from_str::<Value>(line) else {
                                continue;
                            };
                            for event in translator.translate(frame) {
                                let is_terminal =
                                    matches!(event, AgentEvent::Done { .. } | AgentEvent::Error { .. });
                                publish_chunk(broker, scope, source_block_id, request_id, &event, is_terminal);
                                if is_terminal {
                                    saw_terminal = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        if saw_terminal {
            return;
        }

        let still_running = blockcontroller::get_block_controller_status(throwaway_block_id)
            .map(|s| s.shellprocstatus == blockcontroller::STATUS_RUNNING)
            .unwrap_or(false);
        if !still_running {
            // The subprocess ended (or was never started) with no
            // translatable terminal frame ever appearing — synthesize one
            // so the frontend's overlay doesn't spin forever.
            publish_chunk(
                broker,
                scope,
                source_block_id,
                request_id,
                &AgentEvent::Error {
                    message: "agent exited without a final response".to_string(),
                },
                true,
            );
            return;
        }

        if tokio::time::Instant::now() >= deadline {
            publish_chunk(
                broker,
                scope,
                source_block_id,
                request_id,
                &AgentEvent::Error {
                    message: "timed out waiting for a response".to_string(),
                },
                true,
            );
            return;
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Publish one `EVENT_BTW_ANSWER_CHUNK`. See `mps::EVENT_BTW_ANSWER_CHUNK`'s
/// own doc comment for the payload shape and scope-string contract.
fn publish_chunk(
    broker: &mps::Broker,
    scope: &str,
    source_block_id: &str,
    request_id: &str,
    event: &AgentEvent,
    done: bool,
) {
    broker.publish(mps::MuxEvent {
        event: mps::EVENT_BTW_ANSWER_CHUNK.to_string(),
        scopes: vec![scope.to_string()],
        sender: String::new(),
        persist: 0,
        data: Some(serde_json::json!({
            "blockId": source_block_id,
            "requestId": request_id,
            "event": event,
            "done": done,
        })),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    // ── Pure helpers ─────────────────────────────────────────────────

    #[test]
    fn btw_scope_matches_the_documented_format() {
        assert_eq!(btw_scope("blk-1", "req-2"), "block:blk-1:btw:req-2");
    }

    #[test]
    fn build_prompt_concatenates_snapshot_and_question() {
        assert_eq!(
            build_prompt("earlier conversation", "what did I just ask?"),
            "earlier conversation\n\nwhat did I just ask?",
        );
    }

    #[test]
    fn build_prompt_with_no_snapshot_is_just_the_question() {
        assert_eq!(build_prompt("", "hello?"), "hello?");
        assert_eq!(build_prompt("   ", "hello?"), "hello?");
    }

    #[test]
    fn throwaway_meta_excludes_every_agent_identity_key() {
        let mut source = MetaMapType::new();
        source.insert("cmd".to_string(), Value::String("claude".to_string()));
        source.insert(
            "cmd:args".to_string(),
            Value::Array(vec![Value::String("-p".to_string())]),
        );
        source.insert("cmd:cwd".to_string(), Value::String("/tmp/work".to_string()));
        source.insert("agentId".to_string(), Value::String("inst-1".to_string()));
        source.insert("agentName".to_string(), Value::String("smike".to_string()));
        source.insert("agentMode".to_string(), Value::String("container".to_string()));
        source.insert("agent:sessionid".to_string(), Value::String("sid-1".to_string()));

        let got = build_throwaway_meta(&source, BtwProvider::Claude, "");

        assert!(!got.contains_key("agentId"));
        assert!(!got.contains_key("agentName"));
        assert!(!got.contains_key("agentMode"));
        assert!(!got.contains_key("agent:sessionid"));
        assert_eq!(got.get("cmd:cwd"), Some(&Value::String("/tmp/work".to_string())));
    }

    #[test]
    fn throwaway_meta_carries_disallowedtools_and_max_turns() {
        let mut source = MetaMapType::new();
        source.insert(
            "cmd:args".to_string(),
            Value::Array(
                strings(&["-p", "--output-format", "stream-json"])
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
        let got = build_throwaway_meta(&source, BtwProvider::Claude, "");
        let args: Vec<String> = match got.get("cmd:args") {
            Some(Value::Array(arr)) => arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            other => panic!("expected cmd:args array, got {other:?}"),
        };
        assert!(args.iter().any(|a| a == "--disallowedTools"));
        assert!(args.iter().any(|a| a == "--max-turns"));
        assert!(!args.iter().any(|a| a == "--resume"));
    }

    #[test]
    fn throwaway_meta_defaults_cmd_and_args_when_source_has_none() {
        let source = MetaMapType::new();
        let got = build_throwaway_meta(&source, BtwProvider::Claude, "");
        assert_eq!(got.get("cmd"), Some(&Value::String("claude".to_string())));
        assert!(got.contains_key("cmd:args"));
    }

    fn args_of(meta: &MetaMapType) -> Vec<String> {
        match meta.get("cmd:args") {
            Some(Value::Array(arr)) => arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            other => panic!("expected cmd:args array, got {other:?}"),
        }
    }

    fn pane(provider: &str, cmd: &str, args: &[&str]) -> MetaMapType {
        let mut source = MetaMapType::new();
        source.insert("agentProvider".to_string(), Value::String(provider.to_string()));
        source.insert("cmd".to_string(), Value::String(cmd.to_string()));
        source.insert(
            "cmd:args".to_string(),
            Value::Array(strings(args).into_iter().map(Value::String).collect()),
        );
        source.insert("agentId".to_string(), Value::String("inst-1".to_string()));
        source
    }

    #[test]
    fn the_provider_comes_from_the_source_pane() {
        assert_eq!(BtwProvider::of(&MetaMapType::new()), Ok(BtwProvider::Claude));
        assert_eq!(BtwProvider::of(&pane("claude", "claude", &[])), Ok(BtwProvider::Claude));
        assert_eq!(BtwProvider::of(&pane("codex", "codex", &[])), Ok(BtwProvider::Codex));
        assert_eq!(BtwProvider::of(&pane("gemini", "gemini", &[])), Ok(BtwProvider::Gemini));
        let err = BtwProvider::of(&pane("kimi", "kimi", &[])).unwrap_err();
        assert!(err.contains("kimi"), "{err}");
    }

    /// A Codex pane's own argv (here the app-server's) is replaced, not
    /// extended: the side question is a tool-less `exec` on the pane's CLI
    /// and model, without the pane's sandbox bypass.
    #[test]
    fn a_codex_pane_gets_a_tool_less_exec_on_its_own_model() {
        let source = pane("codex", "/opt/codex", &["app-server", "--listen", "stdio://", "-m", "gpt-5"]);
        let got = build_throwaway_meta(&source, BtwProvider::Codex, "");
        assert_eq!(got.get("cmd"), Some(&Value::String("/opt/codex".to_string())));
        assert!(!got.contains_key("agentId"));
        let args = args_of(&got);
        assert_eq!(&args[..2], &strings(&["exec", "--json"])[..]);
        assert!(args.iter().any(|a| a == "--ignore-user-config"));
        assert!(args.windows(2).any(|w| w == strings(&["-m", "gpt-5"])));
        assert!(!args.iter().any(|a| a == "app-server" || a.contains("dangerously")));
    }

    #[test]
    fn a_gemini_pane_gets_the_deny_all_policy_instead_of_yolo() {
        let source = pane("gemini", "gemini", &["--output-format", "stream-json", "--yolo", "-p", "", "-m", "gemini-3-pro"]);
        let got = build_throwaway_meta(&source, BtwProvider::Gemini, "/data/btw/policy.toml");
        let args = args_of(&got);
        assert!(args.windows(2).any(|w| w == strings(&["--policy", "/data/btw/policy.toml"])));
        assert!(args.windows(2).any(|w| w == strings(&["-m", "gemini-3-pro"])));
        assert!(!args.iter().any(|a| a == "--yolo"));
    }

    #[test]
    fn the_gemini_policy_file_holds_the_deny_all_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("btw");
        let path = ensure_gemini_policy_in(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), GEMINI_DENY_ALL_TOOLS_POLICY);
        // A tampered policy is put back; no temp files are left behind.
        std::fs::write(&path, "[[rule]]\ntoolName = \"*\"\ndecision = \"allow\"\n").unwrap();
        assert_eq!(ensure_gemini_policy_in(&dir).unwrap(), path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), GEMINI_DENY_ALL_TOOLS_POLICY);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    // ── Integration: the RPC handler itself must not block ─────────────

    #[tokio::test]
    async fn ask_side_question_returns_a_request_id_without_blocking() {
        // Shared in-memory AppState harness (`server/tests.rs`'s
        // `test_state()`, `pub(crate)` for exactly this kind of cross-file
        // RPC-handler test — see also `agent_handlers/mod.rs`'s own
        // `build_state_with_seed`, a fuller-featured sibling of the same
        // pattern).
        let state = crate::server::tests::test_state();

        // A source block with no real CLI configured — the spawned
        // background turn will fail quickly (no `claude` binary resolves
        // to anything meaningful here), which is fine: this test only
        // asserts the RPC's own synchronous contract — mint + return
        // promptly, never block on the turn itself.
        let mut block = Block {
            oid: "src-block".to_string(),
            ..Default::default()
        };
        state.mstore.insert(&mut block).unwrap();

        let (engine, mut rx) = WshRpcEngine::new();
        register(&engine, &state);

        let req_id = format!("test-{}", uuid::Uuid::new_v4());
        let msg = crate::backend::rpc_types::RpcMessage {
            command: COMMAND_ASK_SIDE_QUESTION.to_string(),
            reqid: req_id.clone(),
            data: Some(serde_json::json!({
                "block_id": "src-block",
                "question": "what does this do?",
                "context_snapshot": "",
            })),
            ..Default::default()
        };
        engine.handle_message(msg);

        let resp = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("handler timed out — RPC must not block on the turn")
            .expect("output channel closed");
        assert_eq!(resp.resid, req_id);
        assert!(resp.error.is_empty(), "handler returned error: {}", resp.error);
        let result: AskSideQuestionResult =
            serde_json::from_value(resp.data.unwrap()).expect("response deserialize");
        assert!(!result.request_id.is_empty());
        assert!(uuid::Uuid::parse_str(&result.request_id).is_ok());
    }
}
