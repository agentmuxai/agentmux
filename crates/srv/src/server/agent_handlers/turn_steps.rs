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
    /// The send-copies of the message's pictures, by path, for a provider that takes a picture that way.
    pub image_paths: Vec<String>,
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
) -> Result<(String, Vec<serde_json::Value>, Vec<String>), String> {
    // Attachments (SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.6,
    // SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §7): every provider gets
    // a numbered list of the files appended to the text; Claude's persistent
    // stream-json path also gets images and short PDFs inline. A container
    // agent can't reach a host path, so its copies are rehomed into the
    // pane's working folder (the host side of its workspace bind mount,
    // narko #198 piece 2) and the list points there instead.
    let mut inline_images: Vec<serde_json::Value> = Vec::new();
    let mut image_paths: Vec<String> = Vec::new();
    let message = if attachments.is_empty() {
        message
    } else {
        // Refusals go through surface_refusal like every other early exit
        // here, so the reason survives a pane reload.
        let container_cwd = match attachment_destination(block) {
            Ok(cwd) => cwd,
            Err(msg) => {
                surface_refusal(deps, block_id, msg);
                return Err(msg.to_string());
            }
        };
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
        let by_path = ctrl.as_any().downcast_ref::<blockcontroller::app_server_controller::AppServerController>().is_some();
        let (prepared, pictures) = tokio::task::spawn_blocking(move || {
            use crate::backend::attachments::prompt;
            let (found, missing) = prompt::resolve(&svc, &attachments);
            let (found, missing) = match &container_cwd {
                Some(cwd) => rehome_for_container(found, missing, cwd),
                None => (found, missing),
            };
            let pictures = if by_path { prompt::image_paths(&found) } else { Vec::new() };
            // Read, spend and record the session budget as one locked step.
            let prepared = svc.with_session_inline_budget(
                &session_key,
                Some(&new_session_key),
                limits.claude_session_inline_bytes,
                |budget| {
                    let prepared = prompt::prepare(&found, &missing, inline_max, budget);
                    let spent = prompt::inline_bytes(&prepared.inline);
                    (prepared, spent)
                },
            );
            (prepared, pictures)
        })
        .await
        .map_err(|e| format!("attachments: {e}"))?;
        inline_images = prepared.inline;
        image_paths = pictures;
        crate::backend::attachments::prompt::append(&message, &prepared.list)
    };
    Ok((message, inline_images, image_paths))
}

/// Where this turn's attachments land, or why they can't: `Ok(None)` for a host agent (nothing to
/// rehome — the host path already means something to it); `Ok(Some(cwd))` for a container agent with a
/// working folder to copy into; `Err` for a runtime that refuses attachments and isn't a container (none
/// does today; kept so a future one still refuses cleanly), or a container with no working folder yet.
fn attachment_destination(block: &Block) -> Result<Option<String>, &'static str> {
    let runtime = RuntimeKind::of_block_meta(&block.meta);
    if !runtime.adapter().accepts_attachments() && !runtime.is_container() {
        return Err("Attachments aren't supported for this agent yet.");
    }
    if !runtime.is_container() {
        return Ok(None);
    }
    // A container's workspace bind mount is this turn's `cmd:cwd`, host side (`ContainerMountSpec::for_agent`,
    // container/mounts.rs): with none set yet there is nowhere on the host to put the copies. Expanded
    // the same way `ContainerMountSpec::for_agent` expands it before binding it as `/workspace`
    // (mounts.rs:50) — a `~/...` cwd (what `agent.open` records for an agent with no workspace of its
    // own) must resolve to the exact folder Docker mounted, or the copy lands somewhere nothing reads it
    // from and the list points at a container path that was never written.
    let cwd = crate::backend::obj::meta_get_string(&block.meta, "cmd:cwd", "");
    let cwd = cwd.trim();
    if cwd.is_empty() {
        return Err("Attachments need a working folder, and this agent's pane has none yet.");
    }
    Ok(Some(crate::backend::base::expand_home_dir_safe(cwd).to_string_lossy().into_owned()))
}

/// The subfolder, under a container's working folder, that rehomed attachments live in.
const CONTAINER_ATTACHMENTS_SUBDIR: &str = ".agentmux-attachments";

/// Copy each resolved attachment's send-copy from its host path into `host_cwd` (the pane's working
/// folder) and point it at the container-side path instead (narko #198 piece 2): a container agent can
/// open a file under its workspace mount, never a bare host path. The copy keeps the exact trailing path
/// [`id_bearing_suffix`] finds — `<sha256>.…` for an image/derived file, `named/<sha256>/<realname>` for
/// any other kind — the same two shapes the composer's reload parser (`ID_RE`, `attached-images.ts`) and
/// the image counter (`images_in_text`, `transcript/images.rs`) already recognize (#7434): renaming to
/// the display name instead would make every container attachment unrecognizable to both on reload. A
/// path with neither shape, or a copy that fails, joins `missing`, the same treatment a send-copy swept
/// by retention gets.
fn rehome_for_container(
    found: Vec<crate::backend::attachments::prompt::Resolved>,
    mut missing: Vec<String>,
    host_cwd: &str,
) -> (Vec<crate::backend::attachments::prompt::Resolved>, Vec<String>) {
    use crate::backend::runtime_adapter::MountRole;
    let container_root = RuntimeKind::Container.adapter().map_mount(MountRole::Workspace, host_cwd);
    let host_root = std::path::Path::new(host_cwd).join(CONTAINER_ATTACHMENTS_SUBDIR);
    let mut rehomed = Vec::with_capacity(found.len());
    for mut item in found {
        let Some(suffix) = id_bearing_suffix(&item.path) else {
            tracing::warn!(name = %item.name, path = %item.path.display(), "container attachment: its host path carries no recognizable id");
            missing.push(item.name);
            continue;
        };
        let dest = host_root.join(&suffix);
        let made_room = match dest.parent() {
            Some(parent) => std::fs::create_dir_all(parent),
            None => Ok(()),
        };
        match made_room.and_then(|()| std::fs::copy(&item.path, &dest)) {
            Ok(_) => {
                let suffix = suffix.to_string_lossy().replace('\\', "/");
                item.path = std::path::PathBuf::from(format!("{container_root}/{CONTAINER_ATTACHMENTS_SUBDIR}/{suffix}"));
                rehomed.push(item);
            }
            Err(e) => {
                tracing::warn!(name = %item.name, error = %e, "container attachment: could not copy into the pane's working folder");
                missing.push(item.name);
            }
        }
    }
    (rehomed, missing)
}

/// The trailing part of a send-copy's host path that carries its attachment id: `<sha256>.…` for an
/// image or other derived file (`prompt.rs`'s module doc), or `named/<sha256>/<realname>` for any other
/// kind. `None` for a path with neither shape — a send-copy always has one, so this only trips on a path
/// this function doesn't recognize, which must not be guessed at.
fn id_bearing_suffix(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let file_name = path.file_name()?;
    if let Some(hash_dir) = path.parent() {
        if hash_dir.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "named") {
            if let Some(hash) = hash_dir.file_name().and_then(|n| n.to_str()).filter(|n| is_sha256_hex(n)) {
                return Some(std::path::Path::new("named").join(hash).join(file_name));
            }
        }
    }
    let leading_id = file_name.to_str()?.split('.').next()?;
    is_sha256_hex(leading_id).then(|| std::path::PathBuf::from(file_name))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Dispatch to a persistent controller: it starts (or respawns) the CLI and carries the message on its input.
pub(super) fn dispatch_persistent(turn: Turn<'_>, persistent_ctrl: &blockcontroller::persistent::PersistentSubprocessController) -> Result<(), String> {
    let Turn { deps, block, block_id, base, env_vars, session_id_field, message, message_id, from, joins_turn, record, inline_images, .. } = turn;
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
    app_server_ctrl.send_message_from(turn.message, turn.image_paths, turn.message_id.is_some(), Some(&turn.record))?;
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
