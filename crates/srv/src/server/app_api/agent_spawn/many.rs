// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/spawn_many`, the `Agents` tool's `spawn` with `workers`
//! (`SPEC_COUNTDOWN_BUTTONS_AND_BATCH_AGENT_ACTIONS_2026_10_28.md` §5): several workers at once, one
//! approval for all of them, and all of them or none. Everything that can be checked is checked before
//! the first worker is made (the briefs, the names, the accounts, the leader's limit,
//! the room under the agent cap); a launch that still fails removes the workers already launched.
//! The MCP process sends the briefs only after srv answers that all of them are open.

use agentmux_common::api_types::{SpawnManyRequest, SpawnWorkerSpec};

use super::*;

pub(crate) async fn handle_agent_spawn_many(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<SpawnManyRequest>,
) -> Response {
    let leader = match leader_of(&state, caller.as_deref(), &req.auth) {
        Ok(leader) => leader,
        Err(e) => return (e.status, Json(json!({ "error": e.message }))).into_response(),
    };
    match spawn_many(&state, leader, &req.workers, &req.user_instruction).await {
        Ok(workers) => Json(json!({ "workers": workers })).into_response(),
        Err(e) => (e.status, Json(json!({ "error": e.message }))).into_response(),
    }
}

/// A worker as it has been checked: nothing is made yet.
pub(super) struct Planned {
    pub(super) name: Option<String>,
    pub(super) brief: String,
    pub(super) runtime: WorkerRuntime,
    pub(super) account: WorkerAccount,
}

/// `error` for worker `n` (1-based) of the batch, in the caller's words.
fn of_worker(n: usize, error: SpawnError) -> SpawnError {
    SpawnError { status: error.status, message: format!("worker {n}: {}", error.message) }
}

/// Check every worker without making anything.
fn plan(state: &AppState, leader: &Leader, specs: &[SpawnWorkerSpec]) -> Result<Vec<Planned>, SpawnError> {
    let provider = state.id_store.resolve_effective_provider_id(&leader.def);
    let leader_runtime = state.mstore.agent_last_runtime_get(&leader.def.id).unwrap_or_default();
    let existing: std::collections::HashSet<String> =
        state.mstore.agent_def_list().unwrap_or_default().into_iter().map(|d| d.name.to_lowercase()).collect();
    let mut given = std::collections::HashSet::new();
    let mut planned = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        let n = i + 1;
        let brief = spec.brief.trim();
        if brief.is_empty() {
            return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: worker {n}: a brief is required: the worker's task, its first message")));
        }
        let name = spec.name.as_deref().map(str::trim).filter(|x| !x.is_empty());
        if let Some(name) = name {
            if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
                return Err(refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: worker {n}: a name is at most {MAX_NAME_CHARS} characters, on one line")));
            }
            if existing.contains(&name.to_lowercase()) || !given.insert(name.to_lowercase()) {
                return Err(refuse(StatusCode::CONFLICT, format!("INVALID_ARG: worker {n}: an agent named {name:?} already exists, or is named twice in this call")));
            }
        }
        let runtime = worker_runtime(&provider, &leader_runtime, spec.model.as_deref(), spec.effort.as_deref())
            .map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("INVALID_ARG: worker {n}: {e}")))?;
        let account = choose_account(state, leader, &provider, spec.account.as_deref()).map_err(|e| of_worker(n, e))?;
        planned.push(Planned { name: name.map(str::to_string), brief: brief.to_string(), runtime, account });
    }
    Ok(planned)
}

/// The one question for the whole batch: every worker, on one line each.
pub(super) fn consent_message_many(leader: &str, planned: &[Planned]) -> String {
    let lines: Vec<String> = planned
        .iter()
        .map(|p| {
            let who = p.name.as_deref().map_or_else(|| "a new agent".to_string(), |n| one_line(n, 60));
            let on = p.runtime.model.as_deref().map(|m| format!(" on {}", one_line(m, 40))).unwrap_or_default();
            format!("• {who}{on}, account {}: {}", one_line(&p.account.label, 60), one_line(&p.brief, 140))
        })
        .collect();
    format!(
        "{} wants to start {} new agents, from its own instructions, in panes next to its own:\n\n{}\n\n\
         They spend tokens until they are stopped. Start them?",
        one_line(leader, 60),
        planned.len(),
        lines.join("\n")
    )
}

pub(super) async fn spawn_many(
    state: &AppState,
    leader: Leader,
    specs: &[SpawnWorkerSpec],
    user_instruction: &str,
) -> Result<Vec<SpawnAgentResult>, SpawnError> {
    if specs.is_empty() {
        return Err(refuse(StatusCode::BAD_REQUEST, "INVALID_ARG: workers is empty: give [{brief, name?, model?, effort?, account?}]"));
    }
    let n = specs.len();
    let planned = plan(state, &leader, specs)?;
    let tab_id = super::super::agent_block::resolve_tab_id_or_active(&state.mstore, &leader.block_id)
        .map_err(|e| refuse(StatusCode::CONFLICT, format!("NO_TAB: your pane isn't in a tab: {e}")))?;
    // Held from counting the leader's workers to the last one open, including the wait for the user.
    let lock = leader_lock(&leader.def.id);
    let _one_at_a_time = lock.lock().await;
    let live = live_workers_of(&state.mstore, &leader.def.id);
    if live + n > MAX_LIVE_WORKERS {
        let room = MAX_LIVE_WORKERS.saturating_sub(live);
        return Err(refuse(
            StatusCode::CONFLICT,
            format!(
                "TOO_MANY_WORKERS: you have {live} workers open and asked for {n}; at most {MAX_LIVE_WORKERS} at once (room for {room}). \
                 Close some (Stop pane) when their work is done, or ask for fewer"
            ),
        ));
    }
    // Stand-ins for the workers, which do not exist yet: the room is checked now, before the user is asked.
    let wanted: Vec<crate::server::agent_lifecycle::Wanted> = planned
        .iter()
        .enumerate()
        .map(|(i, p)| crate::server::agent_lifecycle::Wanted {
            key: format!("spawn-many:{}:{i}", uuid::Uuid::new_v4()),
            label: p.name.clone().unwrap_or_else(|| format!("worker {}", i + 1)),
        })
        .collect();
    crate::server::agent_lifecycle::admit_batch(state, &wanted, false).await.map_err(|e| refuse(StatusCode::CONFLICT, e))?;

    let provenance = blockcontroller::get_controller(&leader.block_id).and_then(|c| c.turn_provenance());
    let asked_user = if crate::sagas::self_quit::gate(provenance.as_ref(), user_instruction).is_err() {
        tracing::info!(leader = %leader.def.name, workers = n, "agent.spawn_many: asking the user");
        let question = json!({
            "kind": "consent",
            "title": format!("{} wants to start {n} workers", one_line(&leader.def.name, 60)),
            "message": consent_message_many(&leader.def.name, &planned),
            "checkbox": "",
            "ok_label": "Start them",
            "cancel_label": "Don't",
        });
        ask_consent(state, &leader, question, "workers", "the workers").await?
    } else {
        false
    };

    // The room is claimed now; a refusal starts nothing. The start limit is not charged here: a worker
    // that has to be started for its brief is charged when the brief is delivered, as for one worker.
    crate::server::agent_lifecycle::admit_batch(state, &wanted, true).await.map_err(|e| refuse(StatusCode::CONFLICT, e))?;

    let mut launched: Vec<(AgentDefinition, AgentOpenResult)> = Vec::with_capacity(n);
    // The names the batch asked for: a generated name must not take one a later worker named.
    let asked: std::collections::HashSet<String> = planned.iter().filter_map(|p| p.name.as_ref()).map(|n| n.to_lowercase()).collect();
    for (i, p) in planned.iter().enumerate() {
        let name = p.name.clone().unwrap_or_else(|| next_worker_name(&state.mstore, &leader.def.name, &asked));
        match launch(state, &leader, &name, &tab_id, &p.runtime, &p.account, Some(wanted[i].key.as_str())).await {
            Ok(done) => launched.push(done),
            Err(e) => {
                // `launch` removed the one that failed; take the others back too.
                for (worker, _) in launched.iter().rev() {
                    discard_worker(state, &tab_id, &worker.id).await;
                    crate::server::agent_lifecycle::release_start(state, &worker.id);
                }
                crate::server::agent_lifecycle::release_batch(state, &wanted);
                let removed = launched.len();
                return Err(refuse(
                    e.status,
                    format!("worker {} of {n} could not be opened, so none were started ({removed} already opened {} removed): {}", i + 1, if removed == 1 { "was" } else { "were" }, e.message),
                ));
            }
        }
    }
    crate::server::agent_lifecycle::release_batch(state, &wanted);
    // Their rooms were rekeyed to the workers' own ids as they were made, and the open that followed took no
    // admission, so no claim is left to be used: give them back.
    for (worker, _) in &launched {
        crate::server::agent_lifecycle::release_start(state, &worker.id);
    }
    tracing::info!(leader = %leader.def.name, workers = n, asked_user, "agent.spawn_many: the workers were launched");
    Ok(launched
        .into_iter()
        .zip(planned)
        .map(|((worker, opened), p)| SpawnAgentResult {
            agent_id: worker.id,
            name: worker.name,
            message_to: worker.slug,
            block_id: opened.block_id,
            tab_id: opened.tab_id,
            provider: opened.provider,
            model: p.runtime.model,
            effort: p.runtime.effort,
            account: p.account.label,
            account_id: p.account.id,
            account_note: p.account.note,
            asked_user,
        })
        .collect())
}
