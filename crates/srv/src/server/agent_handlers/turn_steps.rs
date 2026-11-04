// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `run_agent_turn_joining` (input.rs), one function each, in the order it runs them. Moved from it unchanged.

use std::collections::HashMap;
use std::sync::Arc;

use crate::backend::blockcontroller;
use crate::backend::message_origin::{MessageOrigin, RecordOrigin};
use crate::backend::obj::Block;
use crate::backend::rpc_types::AttachmentRef;
use crate::backend::runtime_adapter::{RuntimeAdapter, RuntimeKind};
use crate::backend::spawn_env::build::build_persistent_spawn_env;

use super::input::{AgentTurnDeps, ArmedJoinHint, TurnGate};

/// What a pane's meta says to run.
pub(super) struct SpawnBase {
    pub cli_command: String,
    pub cli_args: Vec<String>,
    pub working_dir: String,
}

/// One turn, ready to hand to the controller that will run it.
pub(super) struct Turn<'a> {
    pub deps: &'a AgentTurnDeps,
    pub block: &'a Block,
    pub block_id: &'a str,
    pub base: SpawnBase,
    pub env_vars: HashMap<String, String>,
    pub session_id_field: String,
    pub message: String,
    pub message_id: Option<String>,
    pub from: MessageOrigin,
    pub joins_turn: Option<u64>,
    pub record: RecordOrigin,
    pub inline_images: Vec<serde_json::Value>,
}

/// Re-read the spawn config from block metadata.
pub(super) fn spawn_base(block: &Block) -> SpawnBase {
    let cli_command = crate::backend::obj::meta_get_string(&block.meta, "cmd", "claude");
    let cli_args: Vec<String> = match block.meta.get("cmd:args") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![
            "-p".to_string(),
            "--input-format".to_string(),
            "stream-json".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ],
    };
    // A pane stored without --model/--effort (saved before the launch fixes, or
    // written by a path that forgot) would spawn on the CLI default; fill what
    // its `agent:runtime` says. Never overrides a flag already there.
    let cli_args = crate::backend::agent_runtime::with_runtime_flags(&block.meta, cli_args);
    let working_dir = crate::backend::obj::meta_get_string(&block.meta, "cmd:cwd", "");
    SpawnBase { cli_command, cli_args, working_dir }
}

/// The spawn environment, after the single-live-instance check and the identity gate.
pub(super) async fn gated_spawn_env(
    deps: &AgentTurnDeps,
    ctrl: &Arc<dyn blockcontroller::Controller>,
    block: &Block,
    block_id: &str,
    env_vars: HashMap<String, String>,
) -> Result<HashMap<String, String>, TurnGate> {
    let AgentTurnDeps { mstore, id_store, identity_store, auth_key, broker, .. } = deps.clone();
    // Identity gate, MuxBus token, reserved wrapper vars, agent identity,
    // git identity, tools PATH — see `build_persistent_spawn_env`'s own doc
    // comment. Broker hand-in lets the identity resolver publish
    // `identity:no-direct-links`, and its OAuth expiry probe
    // `identityaccounts:changed` when it flips a token's status.
    //
    // Before all of that — the env build writes shared identity state — a
    // persistent pane about to spawn checks that no other AgentMux instance
    // on this host runs this agent (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24
    // Phase 1, I9). A refusal is returned as a `TurnGate`, which the caller
    // persists and shows in the pane like any other spawn refusal.
    let admission = match ctrl
        .as_any()
        .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
    {
        Some(p) if p.needs_spawn() => {
            let uid = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
            let name = crate::backend::obj::meta_get_string(&block.meta, "agentName", "");
            p.check_admission(&uid, &name).await
        }
        _ => Ok(()),
    };
    let env_result = match admission {
        Err(refusal) => Err(TurnGate::Admission(refusal)),
        Ok(()) => build_persistent_spawn_env(
            mstore.clone(),
            id_store.clone(),
            identity_store.clone(),
            Some(broker.clone()),
            &block.meta,
            &block_id,
            &auth_key,
            env_vars,
        )
        .await
        .map_err(TurnGate::Identity),
    };
    env_result
}

/// Show a refusal that happened before any process ran in the pane: the raw frame in the output log, plus the
/// classified failure the recovery row reads.
pub(super) fn surface_refusal(deps: &AgentTurnDeps, block_id: &str, message: &str) {
    let AgentTurnDeps { mstore, broker, filestore_gate, event_bus_gate, .. } = deps.clone();
    let error_frame = serde_json::json!({
        "type": "result",
        "is_error": true,
        "subtype": "error_during_execution",
        "error": {"message": format!("[AgentMux] {message}")}
    })
    .to_string();
    // Some(&filestore_gate): the frame must be PERSISTED
    // to the block file, not just live-broadcast —
    // otherwise the error vanishes on pane
    // reload/reconnect (reagent P1, PR #2164 round 2). And to
    // the agent's global zone: a pane with an agentId reads
    // that on reload, so without it the frame still vanished
    // (Phase 5a-3c).
    let global_zone = crate::backend::transcript::append::resolve_global_output_zone(
        &Some(mstore.clone()),
        &block_id,
    );
    crate::backend::blockcontroller::shell::handle_append_block_file(
        &broker,
        &block_id,
        crate::backend::blockcontroller::subprocess::SUBPROCESS_OUTPUT_SUBJECT,
        format!("{error_frame}\n").as_bytes(),
        Some(&filestore_gate),
        global_zone.as_deref(),
    );
    // codex P1, PR #2802: the frame above only lands in
    // the block's raw output log, which the recovery
    // banner does NOT read from — it reads the
    // structured `agent:last_failure` block-meta key
    // (persist_last_failure) + the ephemeral
    // EVENT_AGENT_FAILURE push, same as every POST-spawn
    // exit classification (host_spawn.rs). No process
    // ever ran here, so classify() gets no exit
    // code/stderr/result-frame — just the gate's own
    // Display text, exactly like health.rs's in-band-error
    // reclassification call.
    let gate_failure =
        crate::agents::failure::classify(None, None, message, None);
    crate::backend::blockcontroller::core::persist_last_failure(
        &block_id,
        Some(&gate_failure),
        &Some(mstore.clone()),
        &Some(event_bus_gate.clone()),
    );
    broker.publish_event::<crate::backend::mps_events::AgentFailed>(&gate_failure, vec![format!("block:{}", block_id)], 1);
}

/// The attachments of a message: a numbered list appended to the text, and the inline images for a persistent Claude.
pub(super) async fn prepare_attachments(
    deps: &AgentTurnDeps,
    ctrl: &Arc<dyn blockcontroller::Controller>,
    block: &Block,
    block_id: &str,
    message: String,
    attachments: Vec<AttachmentRef>,
) -> Result<(String, Vec<serde_json::Value>), String> {
    // Attachments (SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.6,
    // SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §7): every provider gets
    // a numbered list of the files appended to the text; Claude's persistent
    // stream-json path also gets images and short PDFs inline. Container
    // agents can't reach host paths yet (their panes copy files into the
    // working folder instead), so they refuse rather than send a list the
    // agent can't open.
    let mut inline_images: Vec<serde_json::Value> = Vec::new();
    let message = if attachments.is_empty() {
        message
    } else {
        // Refusals go through surface_refusal like every other early exit
        // here, so the reason survives a pane reload.
        if !RuntimeKind::of_block_meta(&block.meta).adapter().accepts_attachments() {
            let msg = "Attachments aren't supported for container agents yet.";
            surface_refusal(deps, block_id, msg);
            return Err(msg.to_string());
        }
        let Some(svc) = crate::backend::attachments::get() else {
            let msg = "Image attachments are unavailable: the attachment service isn't running.";
            surface_refusal(deps, block_id, msg);
            return Err(msg.to_string());
        };
        let is_persistent = ctrl
            .as_any()
            .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
            .is_some();
        let limits = svc.limits();
        let inline_max = if is_persistent { limits.claude_inline_max } else { 0 };
        // Claude Code keeps every inline image in its own session transcript,
        // so inline bytes are budgeted per session too (spec §6.6). A new
        // session has no id yet; its first turn counts against the block.
        // Once the id is known, that first turn's count moves to the session.
        let sid = crate::backend::obj::meta_get_string(&block.meta, "agent:sessionid", "");
        let new_session_key = format!("block:{block_id}:new");
        let session_key = if sid.is_empty() { new_session_key.clone() } else { format!("session:{sid}") };
        // Re-derive any send-copy whose fingerprint is stale (a changed send
        // edge) under the service's CPU and memory limits, before the
        // blocking resolve below looks the files up.
        for a in &attachments {
            svc.ensure_derived(&a.id).await;
        }
        let prepared = tokio::task::spawn_blocking(move || {
            use crate::backend::attachments::prompt;
            let (found, missing) = prompt::resolve(&svc, &attachments);
            // Read, spend and record the session budget as one locked step.
            svc.with_session_inline_budget(
                &session_key,
                Some(&new_session_key),
                limits.claude_session_inline_bytes,
                |budget| {
                    let prepared = prompt::prepare(&found, &missing, inline_max, budget);
                    let spent = prompt::inline_bytes(&prepared.inline);
                    (prepared, spent)
                },
            )
        })
        .await
        .map_err(|e| format!("attachments: {e}"))?;
        inline_images = prepared.inline;
        crate::backend::attachments::prompt::append(&message, &prepared.list)
    };
    Ok((message, inline_images))
}

/// Dispatch to a persistent controller: it starts (or respawns) the CLI and carries the message on its input.
pub(super) fn dispatch_persistent(turn: Turn<'_>, persistent_ctrl: &blockcontroller::persistent::PersistentSubprocessController) -> Result<(), String> {
    let Turn { deps, block, block_id, base, env_vars, session_id_field, message, message_id, from, joins_turn, record, inline_images } = turn;
    let SpawnBase { cli_command, cli_args, working_dir } = base;
    // Container agents use per-turn docker exec — incompatible with a
    // long-lived persistent subprocess. Fail loudly instead of silently
    // spawning the CLI on the host.
    if !RuntimeKind::of_block_meta(&block.meta).adapter().accepts_persistent_controller() {
        return Err("container agents require a subprocess controller; this provider uses a persistent controller".to_string());
    }
    // Resume support: a /model (or effort/permission) change
    // respawns the persistent CLI with new flags; pass the
    // resume flag + captured session id so the respawn continues
    // the same conversation. Same meta keys the subprocess path
    // reads below. Without this, switching model on a persistent
    // agent would either no-op (old behavior) or lose context.
    let resume_flag =
        crate::backend::obj::meta_get_string(&block.meta, "agent:resume_flag", "--resume");
    let persisted_session_id =
        crate::backend::obj::meta_get_string(&block.meta, "agent:sessionid", "");
    let config = blockcontroller::persistent::PersistentSpawnConfig {
        cli_command,
        cli_args,
        working_dir,
        env_vars,
        session_id_field,
        resume_flag,
        session_id: persisted_session_id,
        message_id: message_id.clone(),
    };
    let input = crate::backend::blockcontroller::health::TurnInput { from: from.clone(), text: message.clone(), joins_turn };
    if let Err(e) = persistent_ctrl.send_message_with_images_from(message, inline_images, config, Some(input), record) {
        // A single-live-instance refusal from inside the controller (the spawn-time claim losing
        // to another instance or pane, or the pre-turn fence) gets the early check's pane treatment.
        if crate::backend::agent_admission::is_admission_refusal(&e) || blockcontroller::persistent::is_held_elsewhere_error(&e) {
            surface_refusal(deps, block_id, &e);
        }
        return Err(e);
    }
    Ok(())
}

/// Dispatch to a per-turn subprocess controller (a host CLI, or a Docker exec for a container agent).
pub(super) async fn dispatch_subprocess(
    turn: Turn<'_>,
    subprocess_ctrl: &blockcontroller::subprocess::SubprocessController,
    tracker: Option<Arc<crate::backend::blockcontroller::health::TurnActivityTracker>>,
) -> Result<(), String> {
    let Turn { deps, block, block_id, base, mut env_vars, session_id_field, message, message_id, joins_turn, record, .. } = turn;
    let SpawnBase { cli_command, cli_args, working_dir } = base;
    let AgentTurnDeps { mstore, container_manager, broker, filestore_gate, .. } = deps.clone();
    let resume_flag =
        crate::backend::obj::meta_get_string(&block.meta, "agent:resume_flag", "--resume");
    let resume_strategy = crate::backend::obj::meta_get_string(
        &block.meta,
        "agent:resume_strategy",
        if session_id_field == "thread_id" {
            "codex-exec"
        } else if resume_flag.is_empty() {
            "none"
        } else {
            "flag"
        },
    );
    // Antigravity takes its prompt as `-p <prompt>`, not on stdin.
    let prompt_arg_flag =
        crate::backend::obj::meta_get_string(&block.meta, "agent:prompt_arg_flag", "");
    // Picker reattach: the frontend writes the prior
    // block's session id here when launching with
    // `continueOfInstanceId`. spawn_turn hydrates its
    // inner.session_id from this on the first turn so
    // --resume <sid> lands on the very first launch.
    let persisted_session_id =
        crate::backend::obj::meta_get_string(&block.meta, "agent:sessionid", "");

    // Container agent: a Docker socket API exec per turn (P1a: no secrets in argv). Host agent:
    // a regular CLI subprocess. The adapter is the one place the two differ.
    let adapter: Box<dyn RuntimeAdapter> = match RuntimeKind::of_agent_mode(&crate::backend::obj::meta_get_string(&block.meta, "agentMode", "host")) {
        RuntimeKind::Host => RuntimeKind::Host.adapter(),
        RuntimeKind::Container => {
            let cm = container_manager.get_or_explain().await?;
            // Starting the container takes minutes the first time, so it runs in the turn's own task: this
            // handler is dropped after five seconds. The pre-start builds the same one.
            let adapter = super::container_turn::container_adapter_for_block(
                cm,
                block_id,
                &block.meta,
                &mut env_vars,
                &working_dir,
                broker.clone(),
                filestore_gate.clone(),
                mstore.clone(),
            );
            // Env is passed via CreateExecOptions.env (Docker socket API), NOT as -e KEY=VALUE argv
            // args: this prevents CWE-214 exposure. It is filtered per turn, so cmd:cwd (host path)
            // and host-path vars never reach the container, and each queued turn uses its own env.
            Box::new(adapter)
        }
    };
    adapter.materialize(subprocess_ctrl);
    // Cross-process session-lease key (registry::LeaseStore). Only a host turn enforces it.
    let instance_id = crate::backend::obj::meta_get_string(&block.meta, "agentId", "");
    let config = blockcontroller::subprocess::SubprocessSpawnConfig {
        cli_command,
        cli_args,
        working_dir,
        env_vars,
        message,
        resume_flag,
        resume_strategy,
        prompt_arg_flag,
        session_id_field,
        message_id,
        session_id: if persisted_session_id.is_empty() { None } else { Some(persisted_session_id) },
        instance_id,
        record: Some(record),
    };
    let hint = ArmedJoinHint::arm(joins_turn, tracker);
    adapter.launch(subprocess_ctrl, config)?;
    hint.keep();
    Ok(())
}

/// Register the agent with the cloud subscriber and the reactive handler, so cloud-injected messages reach it.
pub(super) fn register_turn_agent(block: &Block, block_id: &str, local_web_url: &str, turn_agent_id: Option<&str>, turn_uid: Option<&str>) {
    // Register with cloud subscriber + reactive handler so cloud-injected
    // messages (e.g. GitHub PR review notifications) reach this agent.
    // Uses agentName (the logical display name, e.g. "smike") as the name
    // binding — matching the namespace used by `handle_reactive_register`
    // in server/reactive.rs (`req.agent_id`) and the delivery path's
    // lowercased `name_to_blocks` map — with this turn's UID as the key.
    // PR bodies embed $AGENTMUX_AGENT_ID, the stable binding (a fork's
    // `loap-3` under `Loap-3`), so both are subscribed (idempotently).
    // As its route name for every use below ("Loap #3" → "Loap-3"), or the
    // controller's own id disagrees with what a jekt resolves to (#2695).
    let display = crate::backend::obj::meta_get_string(&block.meta, "agentName", "");
    let agent_name = crate::backend::reactive::sanitize::route_name(&display).unwrap_or_default();
    if !agent_name.is_empty() {
        // Identity M2: the display binding is `agentName`, the key is
        // the UID carried in this turn's spawn env. A block that spawned
        // before its row existed (continuation eager-resume, spec §4.4.4
        // Q1) is upgraded in place here on its first turn.
        let registered = crate::backend::reactive::handler::get_global_handler()
            .register_agent_full(
                &agent_name,
                &block_id,
                None,
                0,
                turn_agent_id,
                turn_uid,
                "registration.no_uid.register_tail",
            );
        if registered.is_ok() {
            // Refresh the block's OWN captured identity too
            // (reagentx P1, round 2 on #2697): this call runs on
            // EVERY turn, using block.meta["agentName"] as the
            // source of truth — which can diverge from whatever
            // a PersistentSubprocessController captured at its
            // original spawn_process call (rename, reconfigured
            // cmd:env) without the block ever respawning. Without
            // this, inject_message_inner's recipient-identity
            // check (#2695) would compare the current (correct)
            // target_agent against that stale spawn-time value
            // and falsely reject the agent's own, correctly-
            // addressed jekts as an identity mismatch. A no-op
            // for controller types that don't override
            // set_agent_id (e.g. SubprocessController).
            if let Some(ctrl) = crate::backend::blockcontroller::get_controller(block_id) {
                ctrl.set_agent_id(Some(agent_name.clone()));
            }
            if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                let names = crate::backend::reactive::handler::get_global_handler().names_for_block(block_id);
                names.iter().for_each(|n| sub.add_agent(n));
            }
            // Also mirror into the per-channel + host-global
            // shared file registries — this AgentInput/
            // SubprocessController path (unlike ShellController/
            // PersistentSubprocessController) previously only
            // ever registered in the in-memory Tier-1 map,
            // leaving it permanently unreachable via Tier 2/2b
            // cross-instance/cross-channel delivery (reagent P1,
            // third round on PR #2350).
            let data_dir = crate::backend::base::get_mux_data_dir();
            crate::backend::reactive::registry::write(
                &data_dir,
                &agent_name,
                local_web_url,
                &block_id,
            );
            crate::backend::reactive::registry::write_shared_from_env(
                &agent_name,
                local_web_url,
                &block_id,
            );
        }
    }
}

/// App Server owns a persistent thread/turn process and has its own typed
/// protocol path. Keep it ahead of the legacy stream-json branches.
///
/// KNOWN GAP (#3215, tracked for a follow-up, not fixed here): the turn's
/// env vars (including this turn's freshly-resolved identity
/// bindings from inject_identity_env_async) are computed but never reach
/// this branch — send_message carries no env, and the App Server child was
/// already spawned earlier using only command_from_meta's cmd:env snapshot.
/// Unlike the subprocess/persistent controllers, this isn't a missing
/// plumbing call: env vars are fixed at process exec() time, so an
/// already-running App Server process cannot pick up a per-turn identity
/// change at all without being torn down and respawned. Deciding when to
/// force that respawn (e.g. on every identity-binding change vs. only
/// between turns) is a real design question, not a mechanical fix, so it's
/// deliberately left for a follow-up PR rather than rushed in here.
///
/// A message the pane held during a turn joins that turn (turn-model spec §4.3, J3). The persistent controller carries
/// it on the input itself; the others start passes with no labelled input, so their tracker is told just before the
/// dispatch, never earlier: a failure in the preparation must not leave the hint armed for a later message (#4492).
pub(super) fn dispatch_app_server(
    turn: Turn<'_>,
    app_server_ctrl: &blockcontroller::app_server_controller::AppServerController,
    tracker: Option<Arc<crate::backend::blockcontroller::health::TurnActivityTracker>>,
) -> Result<(), String> {
    let hint = ArmedJoinHint::arm(turn.joins_turn, tracker);
    app_server_ctrl.send_message_from(turn.message, turn.message_id.is_some(), Some(&turn.record))?;
    hint.keep();
    Ok(())
}

/// ACP agents (Copilot, OpenClaw, Pi) and Muse Code (MSP): the message
/// becomes a prompt (a turn) on the agent's running session.
/// If the agent's process exited or crashed, this starts it again from
/// the pane's meta; the message waits for the new session.
pub(super) fn dispatch_session_agent(
    turn: Turn<'_>,
    acp_ctrl: &dyn blockcontroller::session_agent::SessionAgent,
    tracker: Option<Arc<crate::backend::blockcontroller::health::TurnActivityTracker>>,
) -> Result<(), String> {
    acp_ctrl.ensure_started(turn.block.meta.clone())?;
    let hint = ArmedJoinHint::arm(turn.joins_turn, tracker);
    acp_ctrl.send_message(turn.message, turn.message_id.as_deref(), Some(&turn.record))?;
    hint.keep();
    Ok(())
}

#[cfg(test)]
#[path = "turn_steps_tests.rs"]
mod turn_steps_tests;
