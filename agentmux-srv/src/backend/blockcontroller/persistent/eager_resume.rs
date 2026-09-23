// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Eager resume: spawning the CLI on controller construction (before the first
//! message) when a prior session id is on file, so the first turn is not
//! charged the full cold-start.

use super::*;

impl PersistentSubprocessController {
    /// `start()`'s eager-resume attempt for a pane with a captured
    /// `agent:sessionid`. Never returns an error — every failure mode here
    /// is "fall back to lazy", not "fail the resync"; see `start()`'s own
    /// doc comment for why.
    pub(super) fn try_eager_resume(&self, block_meta: &super::super::super::obj::MetaMapType, session_id: &str) -> EagerResumeOutcome {
        let (Some(id_store), Some(identity_store)) = (self.id_store.clone(), self.identity_store.clone()) else {
            return EagerResumeOutcome::DeclinedTo("identity stores not configured for this controller");
        };
        if self.auth_key.is_empty() {
            return EagerResumeOutcome::DeclinedTo("no auth key configured for this controller");
        }
        let Some(mstore) = self.mstore.clone() else {
            return EagerResumeOutcome::DeclinedTo("no mstore configured for this controller");
        };

        // `cli_command`/`cli_args`/`working_dir` — same meta keys a live
        // message's spawn config reads. Deliberately NOT
        // `agent_handlers::input`'s own print-mode fallback default for a
        // missing `cmd:args`: that default is defensive for a code path this
        // eager-resume one never actually reaches in practice
        // (`agent_open.rs` always seeds `cmd:args` for every persistent
        // agent at launch time), and guessing the wrong CLI mode for a
        // persistent agent is worse than just not eager-resuming.
        let cli_command = crate::backend::obj::meta_get_string(block_meta, "cmd", "claude");
        let cli_args: Vec<String> = match block_meta.get("cmd:args") {
            Some(serde_json::Value::Array(arr)) => {
                arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
            }
            _ => return EagerResumeOutcome::DeclinedTo("cmd:args meta missing — refusing to guess CLI flags"),
        };
        let working_dir = crate::backend::obj::meta_get_string(block_meta, "cmd:cwd", "");
        let base_env_vars: HashMap<String, String> = match block_meta.get("cmd:env") {
            Some(serde_json::Value::Object(obj)) => {
                obj.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()
            }
            _ => HashMap::new(),
        };
        let resume_flag = crate::backend::obj::meta_get_string(block_meta, "agent:resume_flag", "--resume");
        let session_id_field = crate::backend::obj::meta_get_string(block_meta, "agent:session_id_field", "session_id");

        // Identity gate, MuxBus token, reserved wrapper vars (unconditional
        // overwrite — AGENTMUX_AUTH_KEY/BLOCKID), agent identity, git
        // identity, tools PATH: the SAME function a live message send
        // builds its env with (`agent_handlers::input::
        // build_persistent_spawn_env`), not a second, independent copy.
        // codex P1 on PR #3513 found an earlier revision of this method had
        // built its own partial copy that silently drifted: missing PATH/
        // MuxBus injection entirely, and `entry().or_insert()` instead of
        // an unconditional overwrite for the two reserved variables (so a
        // stale persisted value for either would have survived an eager
        // resume that a live message send would have corrected).
        //
        // `block_in_place` + `block_on`, not `.await` — `start()` is a sync
        // trait method (shared by every non-async controller type), but its
        // CALLER is not: `resync_controller` (this method's only path in)
        // is invoked synchronously and inline from inside
        // `Box::pin(async move { ... })` handler bodies (`websocket.rs`'s
        // `COMMAND_CONTROLLER_RESYNC`) and from `async fn open_agent_impl`
        // (`agent_open.rs`, both call sites). An earlier revision of this
        // comment argued sync-fn-therefore-safe-to-block and reagent P1 on
        // PR #3513 correctly rejected that: `build_persistent_spawn_env`
        // does a real synchronous-equivalent SQLite query plus, for
        // `SecretRef::Keychain` accounts, a blocking D-Bus/keyring read
        // (async only via `spawn_blocking`/`.await` internally) — run on a
        // bare `.await`-less call from here, that starves the tokio worker
        // it lands on for every unrelated task queued behind it, exactly
        // the failure class already named and fixed once in this codebase
        // (`sysinfo.rs`, `identity_auth_spawn.rs`, `websocket.rs`'s own
        // `COMMAND_CONTROLLER_INPUT` handler — see the latter's comment
        // citing incident #1782). Worse here than a one-off: this runs once
        // per persistent pane on `resync_controller`, i.e. potentially many
        // panes at once, on exactly the mass-reconnect-after-restart
        // scenario this whole PR exists to improve. `block_in_place` hands
        // this worker's other queued tasks off to the pool for the
        // duration; `Handle::current().block_on` then drives the async
        // function to completion on this now-isolated thread.
        let block_id = self.block_id.clone();
        let auth_key = self.auth_key.clone();
        let gate_result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                crate::server::agent_handlers::input::build_persistent_spawn_env(
                    mstore,
                    id_store,
                    identity_store,
                    self.broker.clone(),
                    block_meta,
                    &block_id,
                    &auth_key,
                    base_env_vars,
                ),
            )
        });
        let env_vars = match gate_result {
            Ok(env) => env,
            Err(gate_err) => {
                tracing::warn!(
                    block_id = %self.block_id,
                    error = %gate_err,
                    "eager-resume declined: identity/credential spawn gate did not pass — \
                     falling back to lazy (spawns on next message, gated the same way then)"
                );
                return EagerResumeOutcome::DeclinedTo("identity/credential spawn gate did not pass");
            }
        };

        let config = PersistentSpawnConfig {
            cli_command,
            cli_args,
            working_dir,
            env_vars,
            session_id_field,
            resume_flag,
            session_id: session_id.to_string(),
            message_id: None,
        };
        match self.spawn_process(config, None) {
            Ok(()) => EagerResumeOutcome::Spawned,
            Err(e) => {
                tracing::warn!(
                    block_id = %self.block_id,
                    error = %e,
                    "eager-resume declined: spawn failed — falling back to lazy"
                );
                EagerResumeOutcome::DeclinedTo("spawn failed")
            }
        }
    }
}
