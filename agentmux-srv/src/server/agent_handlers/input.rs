// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::sync::Arc;


use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_SUBPROCESS_SPAWN, COMMAND_AGENT_INPUT, COMMAND_AGENT_STOP,
    CommandSubprocessSpawnData, CommandAgentInputData, CommandAgentStopData,
};
use crate::backend::obj::Block;
use crate::backend::blockcontroller;

use super::super::AppState;

pub fn register_agent_input_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // subprocessspawn → spawn agent CLI as subprocess for a single turn
    let wstore_spawn = state.wstore.clone();
    let broker_spawn = state.broker.clone();
    let event_bus_spawn = state.event_bus.clone();
    let filestore_spawn = state.filestore.clone();
    engine.register_handler(
        COMMAND_SUBPROCESS_SPAWN,
        Box::new(move |data, _ctx| {
            let wstore = wstore_spawn.clone();
            let broker = broker_spawn.clone();
            let event_bus = event_bus_spawn.clone();
            let filestore = filestore_spawn.clone();
            Box::pin(async move {
                let cmd: CommandSubprocessSpawnData = serde_json::from_value(data)
                    .map_err(|e| format!("subprocessspawn: {e}"))?;
                tracing::info!(
                    block_id = %cmd.blockid,
                    cli = %cmd.cli_command,
                    "SubprocessSpawn"
                );

                // Get or create a SubprocessController for this block
                let ctrl = match blockcontroller::get_controller(&cmd.blockid) {
                    Some(c) if c.controller_type() == blockcontroller::BLOCK_CONTROLLER_SUBPROCESS => c,
                    _ => {
                        // Create and register a new SubprocessController
                        let ctrl = blockcontroller::subprocess::SubprocessController::new(
                            cmd.tabid.clone(),
                            cmd.blockid.clone(),
                            Some(broker),
                            Some(event_bus),
                            Some(wstore),
                            Some(filestore),
                        );
                        let ctrl = std::sync::Arc::new(ctrl);
                        ctrl.set_self_ref();
                        blockcontroller::register_controller(&cmd.blockid, ctrl.clone());
                        ctrl as std::sync::Arc<dyn blockcontroller::Controller>
                    }
                };

                // Downcast to SubprocessController to call spawn_turn
                let subprocess_ctrl = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
                    .ok_or_else(|| "controller is not a SubprocessController".to_string())?;

                let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                    cli_command: cmd.cli_command,
                    cli_args: cmd.cli_args,
                    working_dir: cmd.working_dir,
                    env_vars: cmd.env_vars,
                    message: cmd.message,
                    resume_flag: "--resume".to_string(),
                    session_id_field: "session_id".to_string(),
                    message_id: None,
                    // Direct-spawn legacy command — caller doesn't
                    // carry a reattach context. Greenfield session id
                    // is None; spawn_turn captures it from CLI stdout
                    // on the first turn as before.
                    session_id: None,
                };
                subprocess_ctrl.spawn_turn(config)?;
                Ok(None)
            })
        }),
    );

    // agentinput → send message to agent (persistent or per-turn subprocess)
    let wstore_ai = state.wstore.clone();
    let id_store_ai = state.id_store.clone();
    // Streaming-bash wrapper auth — clone the per-launch auth_key into the
    // handler's closure so each spawn can inject it into Claude's env.
    // See SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7.
    let auth_key_ai = state.auth_key.clone();
    // Broker — passed into the identity-injection path so the OAuth
    // expiry probe (PR D, spec §4.4) can publish
    // `identitybundlebindings:changed:<bundle_id>` on status change.
    let broker_ai = state.broker.clone();
    // Container manager — None on hosts without Docker; container agents
    // return an error to the caller rather than crashing the server.
    let container_manager_ai = state.container_manager.clone();
    // Filestore — the spawn-gate error frame must be PERSISTED to the
    // block file, not just live-broadcast (reagent P1, PR #2164 round 2).
    let filestore_ai = state.filestore.clone();
    let local_web_url_ai = state.local_web_url.clone();
    engine.register_handler(
        COMMAND_AGENT_INPUT,
        Box::new(move |data, _ctx| {
            let wstore = wstore_ai.clone();
            let id_store = id_store_ai.clone();
            let auth_key = auth_key_ai.clone();
            let broker = broker_ai.clone();
            let container_manager = container_manager_ai.clone();
            let filestore_gate = filestore_ai.clone();
            let local_web_url = local_web_url_ai.clone();
            Box::pin(async move {
                let cmd: CommandAgentInputData = serde_json::from_value(data)
                    .map_err(|e| format!("agentinput: {e}"))?;
                tracing::info!(block_id = %cmd.blockid, "AgentInput");

                let ctrl = blockcontroller::get_controller(&cmd.blockid)
                    .ok_or_else(|| format!("no controller for block {}", cmd.blockid))?;

                // Re-read the spawn config from block metadata
                let block: Block = wstore
                    .get(&cmd.blockid)
                    .map_err(|e| format!("agentinput: load block: {e}"))?
                    .ok_or_else(|| format!("block {} not found", cmd.blockid))?;

                let cli_command = crate::backend::obj::meta_get_string(
                    &block.meta, "cmd", "claude",
                );
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
                let working_dir = crate::backend::obj::meta_get_string(
                    &block.meta, "cmd:cwd", "",
                );
                let mut env_vars: std::collections::HashMap<String, String> = match block.meta.get("cmd:env") {
                    Some(serde_json::Value::Object(obj)) => obj
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect(),
                    _ => std::collections::HashMap::new(),
                };
                // Identity injection: look up the active AgentInstance for
                // this block, resolve its identity_id's bindings, and merge
                // each per-provider env var into the spawn map. Api-key-class
                // failures are logged and skipped — the agent CLI launches
                // with whatever resolved cleanly plus the static cmd:env
                // block. Oauth-class resolution failures are BLOCKING unless
                // the agent opted into ambient login (layer-3 spawn gate,
                // SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §2.2):
                // surface the error in the agent pane (same
                // `error_during_execution` frame the container spawn-failure
                // path uses below) and abort before the CLI is created.
                // See agentmux-srv/src/identity/resolver.rs. Broker
                // hand-in lets the OAuth expiry probe (PR D, spec §4.4)
                // publish `identitybundlebindings:changed:<bundle_id>`
                // when it flips a token's status valid→expired etc.
                env_vars = match crate::identity::resolver::inject_identity_env_async(
                    wstore.clone(),
                    id_store.clone(),
                    Some(broker.clone()),
                    cmd.blockid.clone(),
                    env_vars,
                )
                .await
                {
                    Ok(env) => env,
                    Err(gate) => {
                        let error_frame = serde_json::json!({
                            "type": "result",
                            "is_error": true,
                            "subtype": "error_during_execution",
                            "error": {"message": format!("[AgentMux] {gate}")}
                        })
                        .to_string();
                        // Some(&filestore_gate): the frame must be PERSISTED
                        // to the block file, not just live-broadcast —
                        // otherwise the error vanishes on pane
                        // reload/reconnect (reagent P1, PR #2164 round 2).
                        crate::backend::blockcontroller::shell::handle_append_block_file(
                            &broker,
                            &cmd.blockid,
                            crate::backend::blockcontroller::subprocess::SUBPROCESS_OUTPUT_SUBJECT,
                            format!("{error_frame}\n").as_bytes(),
                            Some(&filestore_gate),
                            None,
                        );
                        return Err(format!("identity spawn gate: {gate}"));
                    }
                };
                // MuxBus cloud token — injects MUXBUS_TOKEN + MUXBUS_COGNITO_DOMAIN
                // if the user has authenticated via muxbus.login. No-op if no
                // credentials are stored. Auto-refreshes if token is nearly expired.
                crate::server::muxbus_handlers::inject_muxbus_env(&id_store, &mut env_vars).await;
                // Streaming-bash wrapper auth + discovery
                // (SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7).
                //
                // 1. AGENTMUX_AUTH_KEY — config.rs:42 removed it from
                //    the process env at startup (security PR #801).
                //    Re-inject for this spawn so the wrapper (running
                //    inside Claude's bash subprocess tree) can
                //    authenticate against the auth_middleware-gated
                //    /agentmux/wps/publish endpoint via X-AuthKey.
                // 2. PATH — prepend the bundled tools/bin dir so
                //    `agentmux-bashwrap.exe` resolves when the
                //    PreToolUse hook (auto-injected by agent_config.rs)
                //    rewrites the command to invoke it. AGENTMUX_LOCAL_URL
                //    is already in the inherited process env (main.rs:498).
                env_vars.insert("AGENTMUX_AUTH_KEY".to_string(), auth_key.clone());
                // Block id so the wrapper can scope its WPS publishes
                // to `block:<id>`. Without this, chunks publish without
                // a scope and the frontend's per-block subscription
                // doesn't receive them.
                env_vars.insert("AGENTMUX_BLOCKID".to_string(), cmd.blockid.clone());
                // Agent display name for MuxBus self-identification.
                // muxbus-client reads AGENTMUX_AGENT_ID (preferred) or AGENT_NAME.
                // Only set if not already present in cmd:env — user-provided values take precedence.
                if !env_vars.contains_key("AGENTMUX_AGENT_ID") {
                    let agent_display_name = crate::backend::obj::meta_get_string(
                        &block.meta, "agentName", "",
                    );
                    if !agent_display_name.is_empty() {
                        env_vars.insert("AGENTMUX_AGENT_ID".to_string(), agent_display_name);
                    }
                }
                // PATH includes BOTH bundled tools dir (portable
                // builds, runtime/tools/bin/) AND user tools dir
                // (~/.agentmux/tools/bin/). bundled is None in dev
                // mode (target/debug exclusion in tool_store), so
                // without user_tools_dir the wrapper wouldn't be on
                // the agent's PATH during `task dev`.
                {
                    let existing = env_vars
                        .get("PATH")
                        .cloned()
                        .or_else(|| std::env::var("PATH").ok())
                        .unwrap_or_default();
                    let sep = if cfg!(windows) { ";" } else { ":" };
                    let mut extras: Vec<String> = Vec::new();
                    if let Some(d) = crate::backend::tool_store::bundled_tools_dir() {
                        if d.exists() {
                            extras.push(d.to_string_lossy().into_owned());
                        }
                    }
                    if let Some(d) = crate::backend::tool_store::user_tools_dir() {
                        if d.exists() {
                            extras.push(d.to_string_lossy().into_owned());
                        }
                    }
                    if !extras.is_empty() {
                        let new_path = format!("{}{}{}", extras.join(sep), sep, existing);
                        env_vars.insert("PATH".to_string(), new_path);
                    }
                }

                let session_id_field = crate::backend::obj::meta_get_string(
                    &block.meta, "agent:session_id_field", "session_id",
                );

                // Try persistent controller first, fall back to subprocess
                if let Some(persistent_ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
                {
                    // Container agents use per-turn docker exec — incompatible with a
                    // long-lived persistent subprocess. Fail loudly instead of silently
                    // spawning the CLI on the host.
                    let agent_mode = crate::backend::obj::meta_get_string(&block.meta, "agentMode", "host");
                    if agent_mode == "container" {
                        return Err("container agents require a subprocess controller; this provider uses a persistent controller".to_string());
                    }
                    // Resume support: a /model (or effort/permission) change
                    // respawns the persistent CLI with new flags; pass the
                    // resume flag + captured session id so the respawn continues
                    // the same conversation. Same meta keys the subprocess path
                    // reads below. Without this, switching model on a persistent
                    // agent would either no-op (old behavior) or lose context.
                    let resume_flag = crate::backend::obj::meta_get_string(
                        &block.meta, "agent:resume_flag", "--resume",
                    );
                    let persisted_session_id = crate::backend::obj::meta_get_string(
                        &block.meta, "agent:sessionid", "",
                    );
                    let config = blockcontroller::persistent::PersistentSpawnConfig {
                        cli_command,
                        cli_args,
                        working_dir,
                        env_vars,
                        session_id_field,
                        resume_flag,
                        session_id: persisted_session_id,
                        message_id: cmd.message_id.clone(),
                    };
                    persistent_ctrl.send_message(cmd.message, config)?;
                } else if let Some(subprocess_ctrl) = ctrl
                    .as_any()
                    .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
                {
                    let resume_flag = crate::backend::obj::meta_get_string(
                        &block.meta, "agent:resume_flag", "--resume",
                    );
                    // Picker reattach: the frontend writes the prior
                    // block's session id here when launching with
                    // `continueOfInstanceId`. spawn_turn hydrates its
                    // inner.session_id from this on the first turn so
                    // --resume <sid> lands on the very first launch.
                    let persisted_session_id = crate::backend::obj::meta_get_string(
                        &block.meta, "agent:sessionid", "",
                    );

                    // Container agent branch: use Docker socket API exec (P1a: no
                    // secrets in argv). Host agent branch: regular CLI subprocess.
                    let agent_mode = crate::backend::obj::meta_get_string(
                        &block.meta, "agentMode", "host",
                    );
                    if agent_mode == "container" {
                        let cm = container_manager.get().await
                            .ok_or_else(|| "Docker not available on this host; cannot start container agent".to_string())?;
                        let container_image = {
                            let img = crate::backend::obj::meta_get_string(&block.meta, "agent:container_image", "");
                            if img.is_empty() { "ghcr.io/agentmuxai/agent-claude:latest".to_string() } else { img }
                        };
                        // Use agentId (UUID) — always valid as a Docker name; display names can have spaces.
                        let agent_id = crate::backend::obj::meta_get_string(
                            &block.meta, "agentId", "",
                        );
                        let container_name = crate::backend::container::container_name_for_slug(&agent_id);
                        let volumes_json = crate::backend::obj::meta_get_string(
                            &block.meta, "agent:container_volumes", "[]",
                        );
                        let volumes: Vec<String> = serde_json::from_str(&volumes_json).unwrap_or_default();

                        // Ensure container is alive (pull image if needed — P1b).
                        if let Err(e) = cm.ensure_running(&container_name, &container_image, &volumes, &[]).await {
                            // Surface the error in the agent pane before returning, so the user
                            // sees why the container failed (image not found, Docker down, etc.).
                            let error_frame = serde_json::json!({
                                "type": "result",
                                "is_error": true,
                                "subtype": "error_during_execution",
                                "error": {"message": format!("[AgentMux] container ensure_running failed: {e}")}
                            }).to_string();
                            crate::backend::blockcontroller::shell::handle_append_block_file(
                                &broker,
                                &cmd.blockid,
                                crate::backend::blockcontroller::subprocess::SUBPROCESS_OUTPUT_SUBJECT,
                                format!("{error_frame}\n").as_bytes(),
                                None,
                                None,
                            );
                            return Err(format!("container ensure_running failed: {e}"));
                        }

                        tracing::info!(
                            block_id = %cmd.blockid,
                            container = %container_name,
                            image = %container_image,
                            "container agent turn: bollard exec (env via Docker socket, not argv)",
                        );

                        // Env is passed via CreateExecOptions.env (Docker socket API),
                        // NOT as -e KEY=VALUE argv args — this prevents CWE-214 exposure.
                        // spawn_container_turn filters config.env_vars (denylist) per
                        // turn, so cmd:cwd (host path) and host-path vars never reach
                        // the container, and each queued turn uses its own env.

                        // Base cmd: [container_command, ...cli_args]. The command
                        // is the provider CLI resolved INSIDE the image (on PATH,
                        // e.g. `claude`) — NOT `cli_command`/`cmd`, which is the
                        // host-resolved absolute npm path and does not exist in the
                        // container (docker exec would fail "no such file or
                        // directory"). cli_args are format flags (-p, --input-format
                        // …) + provider flags — no host paths, safe as-is.
                        // spawn_container_turn appends --resume <sid> internally.
                        let container_command = crate::backend::obj::meta_get_string(
                            &block.meta, "agent:container_command", "claude",
                        );
                        let mut base_cmd = vec![container_command];
                        base_cmd.extend(cli_args);

                        let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                            cli_command: String::new(), // unused by spawn_container_turn
                            cli_args: vec![],           // unused by spawn_container_turn
                            working_dir: String::new(), // unused — container has own cwd
                            env_vars,
                            message: cmd.message,
                            resume_flag,
                            session_id_field,
                            message_id: cmd.message_id,
                            session_id: if persisted_session_id.is_empty() {
                                None
                            } else {
                                Some(persisted_session_id)
                            },
                        };
                        subprocess_ctrl.spawn_container_turn(cm.clone(), container_name, base_cmd, config)?;
                    } else {
                        // Host agent: regular CLI subprocess (env set on child process, not in argv).
                        let config = blockcontroller::subprocess::SubprocessSpawnConfig {
                            cli_command,
                            cli_args,
                            working_dir,
                            env_vars,
                            message: cmd.message,
                            resume_flag,
                            session_id_field,
                            message_id: cmd.message_id,
                            session_id: if persisted_session_id.is_empty() {
                                None
                            } else {
                                Some(persisted_session_id)
                            },
                        };
                        subprocess_ctrl.spawn_turn(config)?;
                    }
                } else {
                    return Err("controller is not a SubprocessController or PersistentSubprocessController".to_string());
                }

                // Register with cloud subscriber + reactive handler so cloud-injected
                // messages (e.g. GitHub PR review notifications) reach this agent.
                // Uses agentName (the logical display name, e.g. "smike") as the key —
                // matching the namespace used by reactive.rs:233 (`req.agent_id`) and the
                // delivery path (`agent_to_block` keyed by lowercased logical agent_id).
                // PR bodies embed $AGENTMUX_AGENT_ID (same value) so the cloud injection
                // key and the poll key are always consistent.
                // Both calls are idempotent: add_agent skips the WS send if already
                // subscribed; register_agent replaces any stale mapping from a prior session.
                let agent_name = crate::backend::obj::meta_get_string(
                    &block.meta, "agentName", "",
                );
                if !agent_name.is_empty() {
                    let registered = crate::backend::reactive::handler::get_global_handler()
                        .register_agent(&agent_name, &cmd.blockid, None);
                    if registered.is_ok() {
                        if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
                            sub.add_agent(&agent_name);
                        }
                        // Also mirror into the per-channel + host-global
                        // shared file registries — this AgentInput/
                        // SubprocessController path (unlike ShellController/
                        // PersistentSubprocessController) previously only
                        // ever registered in the in-memory Tier-1 map,
                        // leaving it permanently unreachable via Tier 2/2b
                        // cross-instance/cross-channel delivery (reagent P1,
                        // third round on PR #2350).
                        let data_dir = crate::backend::base::get_wave_data_dir();
                        crate::backend::reactive::registry::write(
                            &data_dir,
                            &agent_name,
                            &local_web_url,
                            &cmd.blockid,
                        );
                        crate::backend::reactive::registry::write_shared_from_env(
                            &agent_name,
                            &local_web_url,
                            &cmd.blockid,
                        );
                    }
                }

                Ok(None)
            })
        }),
    );

    // agentstop → stop the running agent subprocess
    engine.register_handler(
        COMMAND_AGENT_STOP,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandAgentStopData = serde_json::from_value(data)
                    .map_err(|e| format!("agentstop: {e}"))?;
                tracing::info!(block_id = %cmd.blockid, force = cmd.force, "AgentStop");
                match blockcontroller::get_controller(&cmd.blockid) {
                    Some(ctrl) => {
                        ctrl.stop(!cmd.force, blockcontroller::STATUS_DONE)?;
                        // Deregister: unregister_block cleans up both agent_to_block and
                        // block_to_agent maps; remove_agent then removes the cloud poll entry
                        // using the logical agent_id recovered from block_to_agent.
                        let handler = crate::backend::reactive::handler::get_global_handler();
                        let agent_name = handler.agent_id_for_block(&cmd.blockid);
                        handler.unregister_block(&cmd.blockid);
                        if let Some(ref name) = agent_name {
                            // Symmetric teardown for the registry writes added
                            // alongside SubprocessSpawn's register_agent call.
                            let data_dir = crate::backend::base::get_wave_data_dir();
                            crate::backend::reactive::registry::remove(&data_dir, name);
                            crate::backend::reactive::registry::remove_shared_from_env(name);
                        }
                        if let (Some(sub), Some(name)) = (
                            crate::muxbus::cloud_subscriber::get_global_subscriber(),
                            agent_name,
                        ) {
                            sub.remove_agent(&name);
                        }
                        Ok(None)
                    }
                    None => Ok(None),
                }
            })
        }),
    );
}
