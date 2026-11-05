// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::backend::blockcontroller;
use crate::backend::obj::Block;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    AttachmentRef, CommandAgentInputData, CommandAgentStopData, CommandSubprocessSpawnData, COMMAND_AGENT_INPUT,
    COMMAND_AGENT_STOP, COMMAND_SUBPROCESS_SPAWN, RpcContext,
};

use super::super::AppState;
use super::turn_steps as steps;


use crate::backend::spawn_env::block_identity::{
    carry_agent_uid_env, carry_agent_workdir_env, deleted_agent_of_rowless_block, persisted_agent_identity,
    split_agent_identity_env,
};
#[cfg(test)]
use crate::backend::spawn_env::block_identity::{carry_block_identity_env, PersistedAgentIdentity};

/// The slice of [`AppState`] an agent turn needs in order to be started.
///
/// Exists so [`run_agent_turn`] can be driven from somewhere other than the
/// `agentinput` RPC handler it was originally inlined in. The second caller is
/// the reactive handler's message sender (`bootstrap::install_agent_turn_delivery`):
/// a `SubprocessController` starts a turn ONLY via this path, so without a
/// non-RPC entry point every inter-agent message addressed to one was dropped.
/// See `docs/reports/REPORT_JEKT_DELIVERY_DROPS_SUBPROCESS_AGENTS_2026_09_02.md`.
///
/// Field names match the local bindings the extracted body already used, so the
/// body itself is unchanged by the extraction.
#[derive(Clone)]
pub struct AgentTurnDeps {
    pub mstore: Arc<crate::backend::storage::store::Store>,
    pub id_store: Arc<crate::backend::storage::store::Store>,
    pub identity_store: Arc<crate::backend::storage::store::Store>,
    /// Streaming-bash wrapper auth key — see SPEC_STREAMING_BASH_RUNNER_2026_05_11.md §7.
    pub auth_key: String,
    pub broker: Arc<crate::backend::mps::Broker>,
    pub container_manager: Arc<crate::backend::container::ContainerRuntimeHandle>,
    /// Named `filestore_gate` because the spawn-gate error frame MUST be
    /// persisted through it, not merely live-broadcast (reagent P1, PR #2164).
    pub filestore_gate: Arc<crate::backend::storage::filestore::FileStore>,
    pub local_web_url: String,
    pub event_bus_gate: Arc<crate::backend::eventbus::EventBus>,
}

impl AgentTurnDeps {
    pub fn from_state(state: &AppState) -> Self {
        Self {
            mstore: state.mstore.clone(),
            id_store: state.id_store.clone(),
            identity_store: state.identity_store.clone(),
            auth_key: state.auth_key.clone(),
            broker: state.broker.clone(),
            container_manager: state.container_manager.clone(),
            filestore_gate: state.filestore.clone(),
            local_web_url: state.local_web_url.clone(),
            event_bus_gate: state.event_bus.clone(),
        }
    }
}

/// Start one agent turn for `block_id` with `message`.
///
/// This is the whole of what `AgentInputCommand` does, verbatim: re-read the
/// spawn config from block meta, inject identity/muxbus/bashwrap env (failing
/// closed on the oauth spawn gate), then dispatch to the persistent controller,
/// the host subprocess path, or the container `docker exec` path as the block's
/// `agentMode` and controller type require — and finally re-register the agent
/// for reactive delivery.
///
/// Extracted from the `agentinput` handler body with no behavior change so a
/// second, non-RPC caller can reach it; see [`AgentTurnDeps`].
/// Whether starting a turn should also (re-)register the agent for reactive
/// delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnRegistration {
    /// The RPC / UI path. Registers, exactly as `AgentInputCommand` always has.
    Register,
    /// The reactive-delivery path (`bootstrap::install_agent_turn_delivery`).
    /// Skipping is required, for two independent reasons:
    ///
    /// 1. **Redundant.** That caller resolved this block *by looking the agent
    ///    up in the reactive handler's own registry* (`uid_to_block` /
    ///    `name_to_blocks`), so the agent
    ///    is registered by construction. There is nothing to re-register.
    /// 2. **Deadlock.** `ReactiveHandler::inject_message` holds the global
    ///    `Mutex<Handler>` across the message-sender call, and that sender
    ///    drives this function to completion on the same thread. The
    ///    registration below calls `get_global_handler().register_agent(...)`,
    ///    which locks that same `std::sync::Mutex` — which is NOT reentrant.
    ///    Registering here would block the thread on a lock it already holds,
    ///    wedging the reactive handler process-wide, on essentially every
    ///    successful delivery to a subprocess agent (reagent P0 on PR #2930).
    Skip,
}

/// Why `run_agent_turn` refused to spawn, before any process started.
pub(super) enum TurnGate {
    /// Another AgentMux instance on this host runs this agent.
    Admission(String),
    /// The identity/credential spawn gate refused.
    Identity(crate::identity::resolver::SpawnGateError),
}

impl std::fmt::Display for TurnGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnGate::Admission(refusal) => f.write_str(refusal),
            TurnGate::Identity(e) => write!(f, "{e}"),
        }
    }
}

/// A held message's join hint, armed for one dispatch. Dropped without
/// `keep` (the dispatch failed), it is cleared, so no later message can take
/// it (#4492).
pub(super) struct ArmedJoinHint(Option<std::sync::Arc<crate::backend::blockcontroller::health::TurnActivityTracker>>);

impl ArmedJoinHint {
    pub(super) fn arm(
        joins_turn: Option<u64>,
        tracker: Option<std::sync::Arc<crate::backend::blockcontroller::health::TurnActivityTracker>>,
    ) -> Self {
        match (joins_turn, tracker) {
            (Some(turn_id), Some(tracker)) => {
                tracker.hint_join(turn_id);
                Self(Some(tracker))
            }
            _ => Self(None),
        }
    }

    /// The dispatch went through: the pass it starts takes the hint.
    pub(super) fn keep(mut self) {
        self.0 = None;
    }
}

impl Drop for ArmedJoinHint {
    fn drop(&mut self) {
        if let Some(tracker) = self.0.take() {
            tracker.clear_join_hint();
        }
    }
}

pub async fn run_agent_turn(
    deps: &AgentTurnDeps,
    block_id: String,
    message: String,
    message_id: Option<String>,
    registration: TurnRegistration,
    from: crate::backend::message_origin::MessageOrigin,
    attachments: Vec<AttachmentRef>,
) -> Result<(), String> {
    run_agent_turn_joining(deps, block_id, message, message_id, registration, from, attachments, None).await
}

/// [`run_agent_turn`] for a message the pane held while turn `joins_turn`
/// ran: if that turn is still settling (or only just ended), the message
/// joins it rather than starting a new one (turn-model spec §4.3, J3).
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_turn_joining(
    deps: &AgentTurnDeps,
    block_id: String,
    message: String,
    message_id: Option<String>,
    registration: TurnRegistration,
    from: crate::backend::message_origin::MessageOrigin,
    attachments: Vec<AttachmentRef>,
    joins_turn: Option<u64>,
) -> Result<(), String> {
    let origin = from.turn_origin();

    let ctrl = blockcontroller::get_controller(&block_id)
        .ok_or_else(|| format!("no controller for block {}", block_id))?;

    // Re-read the spawn config from block metadata
    let block: Block = deps.mstore
        .get(&block_id)
        .map_err(|e| format!("agentinput: load block: {e}"))?
        .ok_or_else(|| format!("block {} not found", block_id))?;
    let base = steps::spawn_base(&block);
    let env_vars = crate::backend::blockcontroller::cmd_env_of(&block.meta);
    let env_result = steps::gated_spawn_env(deps, &ctrl, &block, &block_id, env_vars).await;
    let env_vars = match env_result {
        Ok(env) => env,
        Err(gate) => {
            steps::surface_refusal(deps, &block_id, &gate.to_string());
            return Err(match &gate {
                TurnGate::Admission(refusal) => refusal.clone(),
                TurnGate::Identity(e) => format!("identity spawn gate: {e}"),
            });
        }
    };
    // Identity M2: the UID this turn's spawn env carries (M1a), kept for the
    // Register-tail below so registration is keyed by identity without a
    // second store read (spec §4.4.1 row 2). `None` when the block has no
    // row yet — the tail then registers by name, counted.
    let turn_uid: Option<String> = env_vars
        .get("AGENTMUX_AGENT_UID")
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    let turn_agent_id = env_vars.get("AGENTMUX_AGENT_ID").cloned(); // the stable binding, as spawn parks it

    let session_id_field =
        crate::backend::obj::meta_get_string(&block.meta, "agent:session_id_field", "session_id");

    let (message, inline_images) = steps::prepare_attachments(deps, &ctrl, &block, &block_id, message, attachments).await?;
    let (message, held_notes) = crate::backend::reactive::held_notes::attach_for_turn(&block_id, &message, origin);
    let record = crate::backend::message_origin::RecordOrigin { origin: from.clone(), notes: held_notes.note_texts() };
    let turn = steps::Turn { deps, block: &block, block_id: &block_id, base, env_vars, session_id_field, message, message_id, from, joins_turn, record, inline_images };

    if let Some(app_server_ctrl) =
        ctrl.as_any()
            .downcast_ref::<blockcontroller::app_server_controller::AppServerController>()
    {
        steps::dispatch_app_server(turn, app_server_ctrl, ctrl.turn_tracker())?;
    }
    // Try persistent controller first, fall back to subprocess
    else if let Some(persistent_ctrl) =
        ctrl.as_any()
            .downcast_ref::<blockcontroller::persistent::PersistentSubprocessController>()
    {
        steps::dispatch_persistent(turn, persistent_ctrl)?;
    } else if let Some(subprocess_ctrl) =
        ctrl.as_any()
            .downcast_ref::<blockcontroller::subprocess::SubprocessController>()
    {
        steps::dispatch_subprocess(turn, subprocess_ctrl, ctrl.turn_tracker()).await?;
    } else if let Some(acp_ctrl) =
        blockcontroller::session_agent::session_agent(ctrl.as_ref())
    {
        steps::dispatch_session_agent(turn, acp_ctrl, ctrl.turn_tracker())?;
    } else {
        return Err(
            "controller is not a SubprocessController, PersistentSubprocessController or session agent"
                .to_string(),
        );
    }
    held_notes.delivered();

    // Registration is skipped on the reactive-delivery path — see
    // `TurnRegistration::Skip` for why that is both redundant AND a hard
    // deadlock requirement, not an optimisation.
    if matches!(registration, TurnRegistration::Register) {
        steps::register_turn_agent(&block, &block_id, &deps.local_web_url, turn_agent_id.as_deref(), turn_uid.as_deref());
    }

    Ok(())
}

pub fn register_agent_input_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // subprocessspawn → spawn agent CLI as subprocess for a single turn
    let state_spawn = state.clone();
    engine.register_typed(COMMAND_SUBPROCESS_SPAWN, move |cmd: CommandSubprocessSpawnData, _ctx| {
        let state = state_spawn.clone();
        async move { subprocess_spawn(&state, cmd) }
    });

    // agentinput → send message to agent (persistent or per-turn subprocess).
    // Everything the turn needs from AppState now travels as one
    // `AgentTurnDeps` (see its doc comment for why it is a named struct rather
    // than nine separate closure captures).
    let deps_ai = AgentTurnDeps::from_state(state);
    engine.register_typed(COMMAND_AGENT_INPUT, move |cmd: CommandAgentInputData, ctx| {
        let deps = deps_ai.clone();
        async move { agent_input(&deps, &ctx, cmd).await }
    });

    // agentstop → stop the running agent subprocess
    engine.register_typed(COMMAND_AGENT_STOP, |cmd: CommandAgentStopData, _ctx| async move { agent_stop(cmd) });
}

/// `subprocessspawn`: run the CLI for one turn in the block's subprocess controller, creating it if needed.
pub(crate) fn subprocess_spawn(state: &AppState, cmd: CommandSubprocessSpawnData) -> Result<(), String> {
    let (mstore, broker, event_bus, filestore, boot_id) = (
        state.mstore.clone(),
        state.broker.clone(),
        state.event_bus.clone(),
        state.filestore.clone(),
        state.boot_id.clone(),
    );

    tracing::info!(
        block_id = %cmd.blockid,
        cli = %cmd.cli_command,
        "SubprocessSpawn"
    );

    // Get or create a SubprocessController for this block
    let ctrl = match blockcontroller::get_controller(&cmd.blockid) {
        Some(c)
            if c.controller_type() == blockcontroller::BLOCK_CONTROLLER_SUBPROCESS =>
        {
            c
        }
        _ => {
            // Create and register a new SubprocessController
            let registry = mstore.shared_agent_registry();
            let ctrl = blockcontroller::subprocess::SubprocessController::new(
                cmd.tabid.clone(),
                cmd.blockid.clone(),
                Some(broker),
                Some(event_bus),
                Some(mstore),
                Some(filestore),
                registry,
                boot_id,
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
        resume_strategy: "flag".to_string(),
        prompt_arg_flag: String::new(),
        session_id_field: "session_id".to_string(),
        message_id: None,
        // Direct-spawn legacy command — caller doesn't
        // carry a reattach context. Greenfield session id
        // is None; spawn_turn captures it from CLI stdout
        // on the first turn as before.
        session_id: None,
        // COMMAND_SUBPROCESS_SPAWN has no block to read
        // agentId from (dead code — unused by the
        // frontend, per grep); empty disables leasing.
        instance_id: String::new(),
        record: None,
    };
    subprocess_ctrl.spawn_turn(config)?;
    Ok(())
}

/// `agentinput`: a message typed into the composer (or a hidden reinjection), started as a turn.
pub(crate) async fn agent_input(deps: &AgentTurnDeps, ctx: &RpcContext, cmd: CommandAgentInputData) -> Result<(), String> {
    if cmd.prestart == Some(true) {
        return super::container_turn::prestart(deps, cmd.blockid).await;
    }
    // From the connection, before anything else runs (composer_origin.rs).
    let origin = super::composer_origin::composer_origin(ctx, cmd.hidden.unwrap_or(false) || cmd.system.unwrap_or(false))?;
    tracing::info!(block_id = %cmd.blockid, "AgentInput");
    // Authoritative hidden-window gate for ambient digest reads
    // (next_prompt_suggestion / activity_summary / activity_watcher)
    // — set here, not reconstructed from transcript bytes. See
    // crate::ambient::digest::set_hidden_reinjection_active's doc comment.
    crate::ambient::digest::set_hidden_reinjection_active(
        &cmd.blockid,
        cmd.hidden.unwrap_or(false) && !crate::backend::memory::delivery::is_catch_up_text(&cmd.message),
    );
    // A re-delivery srv composed (`memorydelivery:compose`): its
    // card is written once the message has gone to the agent
    // (below), never for one that didn't start.
    let fallback_card = cmd
        .hidden
        .unwrap_or(false)
        .then_some(())
        .and(cmd.delivery_id.as_deref())
        .and_then(|id| crate::server::memory_delivery_handlers::take_fallback_frame(&cmd.blockid, id));
    // A post-compaction memory reinjection also carries the
    // agent's running summary (SPEC_DURABLE_CONVERSATION_MEMORY
    // §4.4, "Compaction reuse").
    let message = if cmd.hidden.unwrap_or(false) {
        crate::backend::continuity_state::with_state_after_compaction(&deps.mstore, &cmd.blockid, cmd.message)
    } else {
        cmd.message
    };
    let block_id = cmd.blockid.clone();
    run_agent_turn_joining(
        deps,
        cmd.blockid,
        message,
        cmd.message_id,
        TurnRegistration::Register,
        crate::backend::message_origin::MessageOrigin::from_composer(origin),
        cmd.attachments.unwrap_or_default(),
        cmd.joins_turn,
    )
    .await?;
    // Right after the hidden message's own transcript line: replay
    // shows this card in its place (parseHistoryLines.ts).
    if let Some(frame) = fallback_card {
        crate::backend::cli_notice::append_to_pane(&deps.broker, &deps.filestore_gate, &deps.mstore, &block_id, &frame);
    }
    Ok(())
}

/// `agentstop`: stop the block's controller, and take its name-keyed side state down.
pub(crate) fn agent_stop(cmd: CommandAgentStopData) -> Result<(), String> {
    tracing::info!(block_id = %cmd.blockid, force = cmd.force, "AgentStop");
    match blockcontroller::get_controller(&cmd.blockid) {
        Some(_) => {
            // The one teardown path, Stop policy (SPEC_AGENT_TEARDOWN_SINGLE_PATH §5).
            crate::sagas::agent_teardown::stop_now(&cmd.blockid, !cmd.force)?;
            // Deregister: unregister_block cleans up both agent_to_block and
            // block_to_agent maps; remove_agent then removes the cloud poll entry
            // using the logical agent_id recovered from block_to_agent.
            let handler = crate::backend::reactive::handler::get_global_handler();
            // Identity M2: a block can be bound under two names
            // (display + stable); tear down the name-keyed side
            // state for EACH, not just the display name — the
            // stable name's file entry was written at spawn and
            // its cloud subscription would otherwise leak.
            let names = handler.unregister_block(&cmd.blockid);
            let data_dir = crate::backend::base::get_mux_data_dir();
            let sub = crate::muxbus::cloud_subscriber::get_global_subscriber();
            for name in &names {
                // Symmetric teardown for the registry writes added
                // alongside SubprocessSpawn's register_agent call.
                crate::backend::reactive::registry::remove(&data_dir, name);
                crate::backend::reactive::registry::remove_shared_from_env(name);
                if let Some(sub) = sub {
                    sub.remove_agent(name);
                }
            }
            // A stop leaves no registration behind, under any name: an entry a turn filed under the
            // block's current name is not one of `names`, and would read as "running" afterwards.
            crate::backend::reactive::registry::remove_for_block(&data_dir, &cmd.blockid, None);
            crate::backend::reactive::registry::remove_shared_for_block_from_env(&cmd.blockid, None);
            Ok(())
        }
        // Stopping a block with no controller is a no-op, not an
        // error: the UI cannot tell "already stopped" from "never
        // started", and either way the caller's intent is
        // satisfied.
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {

    // ---- identity M0: split_agent_identity_env ---------------------------
    //
    // Fixture per SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md §9.3:
    // two agents whose names collide under `derive_slug` ("AgentY" and
    // "AGENTY" both lowercase to "agenty"), which `agent_def_insert` gave
    // the suffixed slugs `agenty-2` / `agenty-3`. Every assertion below is
    // made against that pair so a change that only works for one agent
    // cannot pass.

    use super::split_agent_identity_env;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The server launch path (`input.rs` fallback) — `cmd:env` carries
    /// nothing identity-shaped, so both must be filled from what the server
    /// knows. Positive case first (retro §5): prove they are SET before
    /// proving anything about what they are not.
    #[test]
    fn server_path_sets_both_display_and_slug_from_server_knowledge() {
        let mut e = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        split_agent_identity_env(&mut e, "AgentY", Some("agenty-3"));
        assert_eq!(e["AGENTMUX_AGENT_DISPLAY"], "AgentY");
        assert_eq!(e["AGENTMUX_AGENT_SLUG"], "agenty-3");
    }

    /// The frontend launch path (`launchAgentDefinition`) already set both
    /// to the same values the server holds — nothing changes, nothing is
    /// duplicated under a different key.
    #[test]
    fn frontend_path_with_agreeing_values_is_left_as_is() {
        let before = env(&[
            ("AGENTMUX_AGENT_ID", "agenty-2"),
            ("AGENTMUX_AGENT_SLUG", "agenty-2"),
            ("AGENTMUX_AGENT_DISPLAY", "AGENTY"),
        ]);
        let mut e = before.clone();
        split_agent_identity_env(&mut e, "AGENTY", Some("agenty-2"));
        assert_eq!(e, before);
    }

    /// §9.4: this phase must not change what `AGENTMUX_AGENT_ID` holds on
    /// either path. Slug-valued on the frontend path, name-valued on the
    /// server path — both survive byte-for-byte.
    #[test]
    fn agentmux_agent_id_is_never_touched_on_either_path() {
        let mut frontend = env(&[("AGENTMUX_AGENT_ID", "agenty-2")]);
        split_agent_identity_env(&mut frontend, "AGENTY", Some("agenty-2"));
        assert_eq!(frontend["AGENTMUX_AGENT_ID"], "agenty-2");

        let mut server = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        split_agent_identity_env(&mut server, "AgentY", Some("agenty-3"));
        assert_eq!(server["AGENTMUX_AGENT_ID"], "AgentY");

        let mut none = env(&[]);
        split_agent_identity_env(&mut none, "AgentY", Some("agenty-3"));
        assert!(
            !none.contains_key("AGENTMUX_AGENT_ID"),
            "must not invent one either"
        );
    }

    /// The server's knowledge wins over a `cmd:env` snapshot: a rename after
    /// launch moves `block.meta["agentName"]` but not the frozen env copy,
    /// and the persisted slug is the collision-resolved one, not whatever
    /// the frontend computed before the row existed.
    #[test]
    fn server_values_override_stale_cmd_env_snapshots() {
        let mut e = env(&[
            ("AGENTMUX_AGENT_DISPLAY", "AgentY"),
            ("AGENTMUX_AGENT_SLUG", "agenty"),
        ]);
        split_agent_identity_env(&mut e, "AgentY (renamed)", Some("agenty-3"));
        assert_eq!(e["AGENTMUX_AGENT_DISPLAY"], "AgentY (renamed)");
        assert_eq!(e["AGENTMUX_AGENT_SLUG"], "agenty-3");
    }

    /// A quick-launch pane (`launchAgent`, provider key as `agentId`) has no
    /// `db_agents` row and therefore no slug. The display name is still set;
    /// the slug is NOT derived from it — that derivation is the lossy step
    /// spec §1.1 identifies as the defect. A `cmd:env` slug survives as the
    /// fallback.
    #[test]
    fn no_persisted_row_sets_display_only_and_keeps_any_cmd_env_slug() {
        let mut bare = env(&[]);
        split_agent_identity_env(&mut bare, "AgentY", None);
        assert_eq!(bare["AGENTMUX_AGENT_DISPLAY"], "AgentY");
        assert!(!bare.contains_key("AGENTMUX_AGENT_SLUG"));

        let mut with_fallback = env(&[("AGENTMUX_AGENT_SLUG", "agenty-3")]);
        split_agent_identity_env(&mut with_fallback, "AgentY", None);
        assert_eq!(with_fallback["AGENTMUX_AGENT_SLUG"], "agenty-3");
    }

    /// An empty or whitespace display name (a pane with no `agentName` meta
    /// yet) sets nothing and does not clobber a `cmd:env` value with "".
    #[test]
    fn empty_display_name_sets_nothing_and_preserves_cmd_env() {
        let mut bare = env(&[]);
        split_agent_identity_env(&mut bare, "   ", None);
        assert!(!bare.contains_key("AGENTMUX_AGENT_DISPLAY"));

        let mut with_fallback = env(&[("AGENTMUX_AGENT_DISPLAY", "AgentY")]);
        split_agent_identity_env(&mut with_fallback, "", Some("  "));
        assert_eq!(with_fallback["AGENTMUX_AGENT_DISPLAY"], "AgentY");
        assert!(!with_fallback.contains_key("AGENTMUX_AGENT_SLUG"));
    }

    /// The whole point: after the split, the two colliding agents are
    /// distinguishable by BOTH variables, whereas anything derived from the
    /// display name alone is not.
    #[test]
    fn colliding_names_stay_separable_after_the_split() {
        let mut a = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        let mut b = env(&[("AGENTMUX_AGENT_ID", "AGENTY")]);
        split_agent_identity_env(&mut a, "AgentY", Some("agenty-3"));
        split_agent_identity_env(&mut b, "AGENTY", Some("agenty-2"));

        assert_ne!(a["AGENTMUX_AGENT_SLUG"], b["AGENTMUX_AGENT_SLUG"]);
        assert_ne!(a["AGENTMUX_AGENT_DISPLAY"], b["AGENTMUX_AGENT_DISPLAY"]);
        // …and the reason the slug has to come from the store rather than
        // from the name: lowercased, the names are the same string.
        assert_eq!(
            a["AGENTMUX_AGENT_DISPLAY"].to_lowercase(),
            b["AGENTMUX_AGENT_DISPLAY"].to_lowercase(),
        );
    }

    // ---- identity M1: carry_agent_uid_env --------------------------------

    use super::{carry_agent_uid_env, PersistedAgentIdentity};

    fn identity(uid: &str, token: Option<&str>) -> PersistedAgentIdentity {
        PersistedAgentIdentity { uid: uid.to_string(), slug: Some("agenty-3".to_string()), token: token.map(str::to_string), former: Default::default() }
    }

    /// Positive case first: a known row carries both values into the env.
    #[test]
    fn carry_sets_uid_and_token_from_the_row() {
        let mut e = env(&[("AGENTMUX_AGENT_ID", "AgentY")]);
        carry_agent_uid_env(&mut e, Some(&identity("4f3c-a91", Some("tok-a"))));
        assert_eq!(e["AGENTMUX_AGENT_UID"], "4f3c-a91");
        assert_eq!(e["AGENTMUX_AGENT_TOKEN"], "tok-a");
        assert_eq!(e["AGENTMUX_AGENT_ID"], "AgentY", "§9.4: untouched");
    }

    /// Reserved-variable rule: a persisted `cmd:env` carrying ANOTHER
    /// agent's UID and token (pane reuse, copied block) is overwritten, not
    /// respected. For the token this is the difference between an agent
    /// holding its own credential and holding someone else's.
    #[test]
    fn carry_overwrites_stale_cmd_env_values_unconditionally() {
        let mut e = env(&[
            ("AGENTMUX_AGENT_UID", "9b2e-7d4"),
            ("AGENTMUX_AGENT_TOKEN", "tok-of-the-other-agent"),
        ]);
        carry_agent_uid_env(&mut e, Some(&identity("4f3c-a91", Some("tok-a"))));
        assert_eq!(e["AGENTMUX_AGENT_UID"], "4f3c-a91");
        assert_eq!(e["AGENTMUX_AGENT_TOKEN"], "tok-a");
    }

    /// No row (quick-launch pane, lookup failure): both are REMOVED. Leaving
    /// a stale token in place would be worse than having none, and the
    /// server has nothing truthful to put there.
    #[test]
    fn carry_removes_both_when_the_server_knows_no_identity() {
        let mut e = env(&[
            ("AGENTMUX_AGENT_ID", "AgentY"),
            ("AGENTMUX_AGENT_UID", "stale"),
            ("AGENTMUX_AGENT_TOKEN", "stale"),
        ]);
        carry_agent_uid_env(&mut e, None);
        assert!(!e.contains_key("AGENTMUX_AGENT_UID"));
        assert!(!e.contains_key("AGENTMUX_AGENT_TOKEN"));
        assert_eq!(e["AGENTMUX_AGENT_ID"], "AgentY");
    }

    /// A row whose token could not be minted still carries its UID, and a
    /// stale token is removed rather than kept — never a half-truth.
    #[test]
    fn carry_with_uid_but_no_token_sets_uid_and_removes_token() {
        let mut e = env(&[("AGENTMUX_AGENT_TOKEN", "stale")]);
        carry_agent_uid_env(&mut e, Some(&identity("4f3c-a91", None)));
        assert_eq!(e["AGENTMUX_AGENT_UID"], "4f3c-a91");
        assert!(!e.contains_key("AGENTMUX_AGENT_TOKEN"));
    }

    // ---- identity M0/M1: persisted_agent_identity against a real store ---

    use super::persisted_agent_identity;
    use crate::backend::storage::agents::AgentDefinition;
    use crate::backend::storage::store::Store;

    fn agent_def(id: &str, name: &str) -> AgentDefinition {
        AgentDefinition {
            seat: None,
            conversation_visibility:
                crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            // Empty on purpose: `agent_def_insert` derives it from `name`
            // and collision-resolves, exactly as the launch flow does.
            slug: String::new(),
            name: name.to_string(),
            icon: "✦".to_string(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 0,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 0,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        }
    }

    fn block_with_agent_id(store: &Store, block_id: &str, agent_id: &str) {
        let mut block = crate::backend::obj::Block {
            oid: block_id.to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: {
                let mut m = crate::backend::obj::MetaMapType::new();
                m.insert("view".to_string(), serde_json::json!("agent"));
                m.insert("agentId".to_string(), serde_json::json!(agent_id));
                m
            },
            subblockids: None,
        };
        store.insert(&mut block).unwrap();
    }

    /// The §1.1 scenario end to end: two agents whose names collide under
    /// `derive_slug` get `agenty` and `agenty-2` from `agent_def_insert`, and
    /// the lookup returns each block's OWN row — its UID, its suffixed slug
    /// (which no derivation from its name could produce), and a token that
    /// is that row's and nobody else's.
    #[test]
    fn persisted_identity_returns_the_rows_uid_slug_and_own_token() {
        let store = Store::open_in_memory().unwrap();
        let mut first = agent_def("uid-first", "AgentY");
        store.agent_def_insert(&mut first).unwrap();
        let mut second = agent_def("uid-second", "AGENTY");
        store.agent_def_insert(&mut second).unwrap();
        assert_eq!(
            first.slug, "agenty",
            "fixture: first insert keeps the bare slug"
        );
        assert_eq!(
            second.slug, "agenty-2",
            "fixture: second insert is suffixed"
        );

        block_with_agent_id(&store, "block-first", "uid-first");
        block_with_agent_id(&store, "block-second", "uid-second");

        let a = persisted_agent_identity(&store, "block-first").expect("first resolves");
        let b = persisted_agent_identity(&store, "block-second").expect("second resolves");
        assert_eq!(a.uid, "uid-first");
        assert_eq!(b.uid, "uid-second");
        assert_eq!(a.slug.as_deref(), Some("agenty"));
        assert_eq!(b.slug.as_deref(), Some("agenty-2"));

        // The token is the persisted one for that UID, minted on this first
        // spawn, and distinct between the two colliding agents.
        let tok_a = a.token.clone().expect("token minted");
        let tok_b = b.token.clone().expect("token minted");
        assert_ne!(tok_a, tok_b);
        assert_eq!(
            store.agent_token_load("uid-first").unwrap().as_deref(),
            Some(tok_a.as_str())
        );
        assert_eq!(
            store.agent_token_load("uid-second").unwrap().as_deref(),
            Some(tok_b.as_str())
        );

        // A second spawn of the same agent carries the SAME token — long-lived
        // for the agent's lifetime (spec §6.3), not re-minted per spawn.
        let a_again = persisted_agent_identity(&store, "block-first").unwrap();
        assert_eq!(a_again.token.as_deref(), Some(tok_a.as_str()));
    }

    /// Identity M4b-2: the helper App Server and ACP use carries the
    /// block's row UID and token into their own env, overwriting a stale
    /// token, and strips both for a block with no row — outside a runtime
    /// and, through `block_in_place`, on a multi-thread runtime worker.
    #[test]
    fn carry_agent_workdir_env_is_the_launch_cwd_for_host_agents_only() {
        let meta = |pairs: &[(&str, &str)]| -> crate::backend::obj::MetaMapType {
            pairs.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect()
        };
        let stale = || {
            std::collections::HashMap::from([("AGENTMUX_AGENT_WORKDIR".to_string(), "C:/stale".to_string())])
        };

        let workdir = |e: &std::collections::HashMap<String, String>| std::path::PathBuf::from(&e["AGENTMUX_AGENT_WORKDIR"]);
        let ws = std::env::temp_dir().join("clamk-0612a");
        let ws_str = ws.to_string_lossy().into_owned();

        for mode in ["host", "standalone"] {
            let mut e = stale();
            super::carry_agent_workdir_env(&mut e, &meta(&[("agentMode", mode), ("cmd:cwd", &ws_str)]));
            assert_eq!(workdir(&e), ws, "{mode}");
        }
        // No agentMode is a host agent, srv's default.
        let mut e = stale();
        super::carry_agent_workdir_env(&mut e, &meta(&[("cmd:cwd", &ws_str)]));
        assert_eq!(workdir(&e), ws);

        // Exported as the absolute path the process starts in: a relative
        // cwd resolves against srv's own cwd, `~` against the home dir.
        let mut e = stale();
        super::carry_agent_workdir_env(&mut e, &meta(&[("cmd:cwd", "projects/foo")]));
        assert_eq!(workdir(&e), std::env::current_dir().unwrap().join("projects/foo"));
        if let Some(home) = dirs::home_dir() {
            let mut e = stale();
            super::carry_agent_workdir_env(&mut e, &meta(&[("cmd:cwd", "~/.agentmux/agents/x-0930a")]));
            assert_eq!(workdir(&e), home.join(".agentmux/agents/x-0930a"));
        }

        let mut e = stale();
        super::carry_agent_workdir_env(&mut e, &meta(&[("agentMode", "container"), ("cmd:cwd", &ws_str)]));
        assert!(!e.contains_key("AGENTMUX_AGENT_WORKDIR"), "container");
        let mut e = stale();
        super::carry_agent_workdir_env(&mut e, &meta(&[("agentMode", "host"), ("cmd:cwd", "  ")]));
        assert!(!e.contains_key("AGENTMUX_AGENT_WORKDIR"), "no cwd");
    }

    #[test]
    fn carry_block_identity_env_carries_or_strips() {
        let store = Store::open_in_memory().unwrap();
        let mut def = agent_def("uid-carry", "Carrier");
        store.agent_def_insert(&mut def).unwrap();
        block_with_agent_id(&store, "block-carry", "uid-carry");
        block_with_agent_id(&store, "block-carry-none", "claude");

        let mut env = std::collections::HashMap::from([(
            "AGENTMUX_AGENT_TOKEN".to_string(),
            "stale-token".to_string(),
        )]);
        super::carry_block_identity_env(&store, "block-carry", &mut env);
        assert_eq!(
            env.get("AGENTMUX_AGENT_UID").map(String::as_str),
            Some("uid-carry")
        );
        assert_eq!(
            env.get("AGENTMUX_AGENT_TOKEN"),
            store.agent_token_load("uid-carry").unwrap().as_ref()
        );

        let mut none = env.clone();
        super::carry_block_identity_env(&store, "block-carry-none", &mut none);
        assert!(
            !none.contains_key("AGENTMUX_AGENT_UID") && !none.contains_key("AGENTMUX_AGENT_TOKEN")
        );

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .unwrap();
        let store = std::sync::Arc::new(store);
        let on_worker = rt.block_on({
            let store = store.clone();
            async move {
                tokio::spawn(async move {
                    let mut env = std::collections::HashMap::new();
                    super::carry_block_identity_env(&store, "block-carry", &mut env);
                    env
                })
                .await
                .unwrap()
            }
        });
        assert_eq!(
            on_worker.get("AGENTMUX_AGENT_UID").map(String::as_str),
            Some("uid-carry")
        );
    }

    /// A quick-launch pane carries a provider key as `agentId` and has no
    /// `db_agents` row; a block that does not exist has nothing either.
    /// Both are "no identity", never an error and never a derived value —
    /// and, load-bearing for the token: no row means no token is minted.
    #[test]
    fn persisted_identity_is_none_for_quick_launch_panes_and_missing_blocks() {
        let store = Store::open_in_memory().unwrap();
        block_with_agent_id(&store, "block-quick", "claude");
        assert_eq!(persisted_agent_identity(&store, "block-quick"), None);
        assert_eq!(persisted_agent_identity(&store, "block-missing"), None);
        assert_eq!(
            store.agent_token_load("claude").unwrap(),
            None,
            "no token minted for a provider key"
        );
    }

    /// Codex P1 on #3591: the builder re-checks deletion at its last read —
    /// an agent deleted after the gate's lookup (no row resolves now, and the
    /// shared registry holds only a retired record) is reported, so the spawn
    /// is refused; a live agent is not.
    #[test]
    fn the_builder_recheck_reports_an_agent_deleted_after_the_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let defs = crate::registry::DefinitionStore::open(tmp.path().join("definitions")).unwrap();
        store.set_def_registry(std::sync::Arc::new(defs));
        let mut def = agent_def("uid-late-del", "LateDel");
        store.agent_def_insert(&mut def).unwrap();
        block_with_agent_id(&store, "block-late-del", "uid-late-del");
        let live = persisted_agent_identity(&store, "block-late-del");
        assert!(live.is_some());
        assert_eq!(
            super::deleted_agent_of_rowless_block(&store, "block-late-del", live.as_ref()),
            None
        );

        assert!(store.agent_def_delete("uid-late-del").unwrap());
        let after = persisted_agent_identity(&store, "block-late-del");
        assert_eq!(after, None);
        assert_eq!(
            super::deleted_agent_of_rowless_block(&store, "block-late-del", after.as_ref())
                .as_deref(),
            Some("uid-late-del")
        );
    }

    /// Spec §6.3: revoked on deletion. Deleting the agent through either
    /// deletion entry point takes its token with it, so a later spawn on a
    /// reused block cannot resurrect the old credential.
    #[test]
    fn deleting_the_agent_revokes_its_token() {
        let store = Store::open_in_memory().unwrap();
        let mut def = agent_def("uid-del", "Deleted");
        store.agent_def_insert(&mut def).unwrap();
        block_with_agent_id(&store, "block-del", "uid-del");
        let minted = persisted_agent_identity(&store, "block-del").unwrap().token;
        assert!(minted.is_some());

        assert!(store.agent_def_delete("uid-del").unwrap());
        assert_eq!(store.agent_token_load("uid-del").unwrap(), None);
        assert_eq!(persisted_agent_identity(&store, "block-del"), None);

        // …and via the launch-side entry point too.
        let mut def2 = agent_def("uid-del2", "Deleted2");
        store.agent_def_insert(&mut def2).unwrap();
        block_with_agent_id(&store, "block-del2", "uid-del2");
        assert!(persisted_agent_identity(&store, "block-del2")
            .unwrap()
            .token
            .is_some());
        assert!(store.instance_delete("uid-del2").unwrap());
        assert_eq!(store.agent_token_load("uid-del2").unwrap(), None);
    }
}

#[cfg(test)]
#[path = "input_handler_tests.rs"]
mod handler_tests;
