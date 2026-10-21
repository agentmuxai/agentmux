// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Terminal` `run` with `secrets`: a command that gets Vault values in its
//! environment without the agent ever holding them
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §7).
//!
//! The agent names `{ENV_NAME: entry}`. srv checks who is asking, asks the person
//! (every window shows the entry, the agent, the command and where it runs; "Always"
//! is a grant per entry and agent), and only then reads the value, straight into
//! that one process's environment. The returned [`Scrubber`] takes the values back
//! out of the output before it is published.

use std::time::Duration;

use axum::http::StatusCode;

use crate::backend::vault::scrub::{self, Scrubber};
use crate::backend::vault::uses::{self, Decision, UseRequest};
use crate::backend::vault::{self, FillTarget};

use super::browser_vault_fill::Waiting;
use super::AppState;

/// Under the 150 s the Terminal tool waits for the job to be created.
const ANSWER_WAIT: Duration = Duration::from_secs(120);
/// Most entries one command may be given.
const MOST: usize = 16;
/// The longest command line, and working directory, the prompt shows whole. A
/// longer one is refused rather than shown cut: the person approves what runs.
const MAX_COMMAND: usize = 2000;
const MAX_CWD: usize = 300;

/// The key a command grant is stored under, in the grants table that web forms
/// use for sites: `command:<agent>:<part>` can't be a site's origin. It is for one
/// part of an entry: a card's expiry is not its number.
pub(crate) fn grant_key(agent: &str, part: &str) -> String {
    format!("command:{agent}:{part}")
}

/// What the command is given and what to take out of its output.
pub(super) struct Injected {
    pub env: Vec<(String, String)>,
    pub scrubber: Scrubber,
}

type Refusal = (StatusCode, String);

fn refuse(code: StatusCode, why: impl Into<String>) -> Refusal {
    (code, why.into())
}

fn env_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.to_ascii_uppercase().starts_with("AGENTMUX_")
}

fn clip(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

/// Resolves, asks for and reads the entries a command was given. `None` when it
/// was given none.
pub(super) async fn inject(
    state: &AppState,
    agent_block_id: &str,
    auth: Option<&agentmux_common::api_types::UiAutomationAuth>,
    wanted: Option<&std::collections::HashMap<String, String>>,
    remote: bool,
    cmd: &str,
    cwd: &str,
) -> Result<Option<Injected>, Refusal> {
    let Some(wanted) = wanted.filter(|w| !w.is_empty()) else { return Ok(None) };
    if remote {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "a Vault secret can't be sent to WSL or an SSH host yet: it would have to cross to another machine; run the command here",
        ));
    }
    if cmd.chars().count() > MAX_COMMAND || cwd.chars().count() > MAX_CWD {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            format!("a command given Vault secrets must fit in the approval prompt ({MAX_COMMAND} characters, folder {MAX_CWD}): put the steps in a script file and run that"),
        ));
    }
    if wanted.len() > MOST {
        return Err(refuse(StatusCode::BAD_REQUEST, format!("a command takes at most {MOST} Vault secrets")));
    }
    // Who is asking, proved by the agent's own signature: the prompt names them, and
    // a grant is theirs alone.
    let agent = super::app_api::connections::verified_agent(state, agent_block_id, auth).map_err(|e| {
        refuse(StatusCode::FORBIDDEN, format!("a Vault secret needs the agent's signed identity: {}", e.replace("an SSH connection needs", "this needs")))
    })?;
    let agent: String = agent.chars().take(64).collect();
    let mut names: Vec<(&String, &String)> = wanted.iter().collect();
    names.sort();
    for (name, _) in &names {
        if !env_name_ok(name) {
            return Err(refuse(StatusCode::BAD_REQUEST, format!("{name:?} isn't a usable environment variable name")));
        }
    }
    let mut targets: Vec<(String, FillTarget)> = Vec::new();
    for (name, entry) in &names {
        let store = state.identity_store.clone();
        let entry = (*entry).clone();
        let target = tokio::task::spawn_blocking(move || vault::command_target(&store, &entry))
            .await
            .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .map_err(|e| refuse(StatusCode::BAD_REQUEST, e))?;
        targets.push(((*name).clone(), target));
    }
    let record = |t: &FillTarget, outcome: &str| {
        vault::record(&state.identity_store, &t.id, &t.label, &agent, "ran a command", &clip(cmd, 160), outcome);
    };

    // One question per entry, covering the parts of it the agent hasn't been allowed
    // to use in commands: a grant is for one part (a card's expiry is not its number).
    let mut asked: Vec<&str> = Vec::new();
    for (_, first) in &targets {
        if asked.contains(&first.id.as_str()) {
            continue;
        }
        asked.push(&first.id);
        let mut waiting_on: Vec<(&str, &FillTarget)> = Vec::new();
        for (name, t) in targets.iter().filter(|(_, t)| t.id == first.id) {
            let (store, id, key) = (state.identity_store.clone(), t.id.clone(), grant_key(&agent, t.part));
            let granted = tokio::task::spawn_blocking(move || store.vault_grant_check(&id, &key).unwrap_or(false)).await.unwrap_or(false);
            if !granted {
                waiting_on.push((name.as_str(), t));
            }
        }
        if waiting_on.is_empty() {
            continue;
        }
        let use_id = uuid::Uuid::new_v4().to_string();
        let mut whats: Vec<&str> = waiting_on.iter().map(|(_, t)| t.what.as_str()).collect();
        whats.dedup();
        let what = whats.join(" and ");
        let request = UseRequest {
            id: use_id.clone(),
            agent: agent.clone(),
            entry_id: first.id.clone(),
            entry_label: first.label.clone(),
            what: what.clone(),
            origin: String::new(),
            field: waiting_on.iter().map(|(n, t)| format!("{n} ({})", t.part)).collect::<Vec<_>>().join(", "),
            offer_always: true,
            reason: format!("it is the first time {agent} runs a command with this entry"),
            needs_value: false,
            kind: first.kind.to_string(),
            command: cmd.to_string(),
            cwd: cwd.to_string(),
            requested_ms: agentmux_common::time::now_ms_u64(),
        };
        let answer = uses::uses().ask(request);
        let _waiting = Waiting::new(use_id, state.clone());
        super::vault_handlers::publish(state);
        let _asking = crate::backend::user_attention::Asking::start(
            &state.broker,
            &state.mstore,
            agent_block_id,
            "vault",
            &format!("{agent} wants to run a command with your {what}"),
        );
        match tokio::time::timeout(ANSWER_WAIT, answer).await {
            Ok(Ok(a)) if a.decision == Decision::Once => {}
            Ok(Ok(a)) if a.decision == Decision::Always => {
                for (_, t) in &waiting_on {
                    let (store, id, key, created) = (state.identity_store.clone(), t.id.clone(), grant_key(&agent, t.part), t.created_at);
                    let _ = tokio::task::spawn_blocking(move || store.vault_grant_add(&id, &key, agentmux_common::time::now_ms(), created)).await;
                }
            }
            Ok(_) => {
                record(first, "denied");
                return Err(refuse(StatusCode::FORBIDDEN, "the person said no: the command did not run"));
            }
            Err(_) => {
                record(first, "timed out");
                return Err(refuse(StatusCode::REQUEST_TIMEOUT, "the person didn't answer in time: the command did not run; ask again when they are there"));
            }
        }
    }

    // Allowed: the values leave their sealed rows now, into this process only.
    let mut env = Vec::new();
    let mut known: Vec<(String, String)> = Vec::new();
    for (name, target) in &targets {
        let (store, id, part, created) = (state.identity_store.clone(), target.id.clone(), target.part, target.created_at);
        let value = match tokio::task::spawn_blocking(move || vault::value_for_fill(&store, &id, created, part)).await {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                record(target, "failed");
                return Err(refuse(StatusCode::BAD_REQUEST, e));
            }
            Err(e) => return Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        };
        if value.len() < scrub::MIN_LEN {
            record(target, "failed");
            return Err(refuse(
                StatusCode::BAD_REQUEST,
                format!("{} is too short to hand to a command: its output couldn't be cleaned of it", target.label),
            ));
        }
        known.push((target.id.clone(), value.to_string()));
        env.push((name.clone(), value.to_string()));
    }
    for (_, target) in &targets {
        record(target, "done");
        let (store, id) = (state.identity_store.clone(), target.id.clone());
        let _ = tokio::task::spawn_blocking(move || store.vault_touch(&id, agentmux_common::time::now_ms())).await;
    }
    Ok(Some(Injected { env, scrubber: Scrubber::new(&known) }))
}

#[cfg(test)]
#[path = "shell_secrets/tests.rs"]
mod tests;
