// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Terminal` `run` with `secrets`, `stdin_secret` and `secret_files`: a command
//! that gets Vault values without the agent ever holding them
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §7,
//! SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.3).
//!
//! The agent names `{ENV_NAME: entry}` for the environment, one entry for stdin, and
//! `{ENV_NAME: entry}` for files whose path goes in `ENV_NAME`. srv checks who is
//! asking, asks the person (every window shows the entry, the agent, the command and
//! where it runs; "Always" is a grant per entry and agent), and only then reads the
//! value, straight into that one process. The returned [`Scrubber`] takes the values
//! back out of the output before it is published.

use std::time::Duration;

use axum::http::StatusCode;
use futures_util::stream::{FuturesUnordered, StreamExt};

use crate::backend::vault::capture;
use crate::backend::vault::scrub::{self, Scrubber};
use crate::backend::vault::uses::{Ended, UseRequest};
use crate::backend::vault::{self, FillTarget};

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
    /// Written to its stdin, then stdin is closed.
    pub stdin: Option<zeroize::Zeroizing<String>>,
    /// `(ENV_NAME, value)`: a file each, whose path goes in `ENV_NAME`.
    pub files: Vec<(String, zeroize::Zeroizing<String>)>,
    pub scrubber: Scrubber,
    /// The entries it was given because nobody answered in 30 s: the agent is
    /// told so.
    pub went_ahead: Vec<String>,
}

/// What a command asks to be given, as the request names it.
#[derive(Default, Clone, Copy)]
pub(super) struct Wanted<'a> {
    pub env: Option<&'a std::collections::HashMap<String, String>>,
    pub stdin: Option<&'a str>,
    pub files: Option<&'a std::collections::HashMap<String, String>>,
    /// The agent's reason for the command's secrets, already checked.
    pub reason: &'a str,
}

/// Where one value goes.
#[derive(Debug, Clone, PartialEq)]
enum Delivery {
    Env(String),
    Stdin,
    File(String),
}

impl Delivery {
    /// How the prompt names it, with the part: `STRIPE_KEY (value)`, `stdin (value)`.
    fn shown(&self, part: &str) -> String {
        match self {
            Delivery::Env(n) => format!("{n} ({part})"),
            Delivery::Stdin => format!("stdin ({part})"),
            Delivery::File(n) => format!("a file at {n} ({part})"),
        }
    }
}

fn sorted(m: Option<&std::collections::HashMap<String, String>>) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = m.map(|m| m.iter().map(|(k, e)| (k.clone(), e.clone())).collect()).unwrap_or_default();
    v.sort();
    v
}

/// Every value the request names, checked: names usable, none twice, at most [`MOST`].
fn deliveries(w: Wanted<'_>) -> Result<Vec<(Delivery, String)>, Refusal> {
    let env = sorted(w.env);
    let files = sorted(w.files);
    for (name, _) in env.iter().chain(files.iter()) {
        if !env_name_ok(name) {
            return Err(refuse(StatusCode::BAD_REQUEST, format!("{name:?} isn't a usable environment variable name")));
        }
    }
    if let Some((name, _)) = files.iter().find(|(f, _)| env.iter().any(|(n, _)| n == f)) {
        return Err(refuse(StatusCode::BAD_REQUEST, format!("{name} is named in both secrets and secret_files")));
    }
    let mut out: Vec<(Delivery, String)> = env.into_iter().map(|(n, e)| (Delivery::Env(n), e)).collect();
    if let Some(entry) = w.stdin.map(str::trim).filter(|s| !s.is_empty()) {
        out.push((Delivery::Stdin, entry.to_string()));
    }
    out.extend(files.into_iter().map(|(n, e)| (Delivery::File(n), e)));
    if out.len() > MOST {
        return Err(refuse(StatusCode::BAD_REQUEST, format!("a command takes at most {MOST} Vault secrets")));
    }
    Ok(out)
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

const DID_NOT_RUN: &str = "the command did not run";

/// Asks the person, in the agent's pane `agent_block_id`, and waits for how it
/// ended: srv decides on silence after [`ANSWER_WAIT`] at most.
pub(super) async fn ask(state: &AppState, agent_block_id: &str, mut request: UseRequest, headline: &str) -> Ended {
    request.block_id = agent_block_id.to_string();
    super::vault_ask::ask(state, agent_block_id, request, ANSWER_WAIT, headline).await.0
}

fn refused(ended: Ended) -> Refusal {
    let code = if ended == Ended::NoAnswer { StatusCode::REQUEST_TIMEOUT } else { StatusCode::FORBIDDEN };
    refuse(code, ended.refusal(DID_NOT_RUN))
}

/// The agent asking, proved by its own signature: a prompt names them, and what
/// they make or are allowed is theirs alone.
fn agent_of(state: &AppState, agent_block_id: &str, auth: Option<&agentmux_common::api_types::UiAutomationAuth>, needs: &str) -> Result<String, Refusal> {
    let agent = super::app_api::connections::verified_agent(state, agent_block_id, auth)
        .map_err(|e| refuse(StatusCode::FORBIDDEN, format!("{needs} needs the agent's signed identity: {}", e.replace("an SSH connection needs", "this needs"))))?;
    Ok(agent.chars().take(64).collect())
}

/// Checks a command's `capture` before it runs (spec §4.2 of the generate spec).
/// A rule that makes a new entry asks nobody: like `generate`, it holds nothing
/// of the person's yet, and the agent could run the command without it and read
/// the value. A rule that replaces an existing entry's value asks the person, each
/// time, naming the command. `None` when there is no capture; with it, the labels
/// of the entries nobody answered for, which went ahead.
pub(super) async fn plan_capture(
    state: &AppState,
    agent_block_id: &str,
    auth: Option<&agentmux_common::api_types::UiAutomationAuth>,
    rules: Option<&[agentmux_common::api_types::CaptureRule]>,
    cmd: &str,
    cwd: &str,
    reason: &str,
) -> Result<(Option<capture::Job>, Vec<String>), Refusal> {
    let Some(rules) = rules.filter(|r| !r.is_empty()) else {
        return Ok((None, Vec::new()));
    };
    let agent = agent_of(state, agent_block_id, auth, "capture")?;
    let (store, rules, a, c) = (state.identity_store.clone(), rules.to_vec(), agent.clone(), cmd.to_string());
    let mut plan = tokio::task::spawn_blocking(move || capture::plan(&store, &rules, &a, &c))
        .await
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| refuse(StatusCode::BAD_REQUEST, e))?;
    plan.reason = reason.to_string();
    let replaced: Vec<(String, String, &'static str, String)> = plan
        .replaces()
        .filter_map(|t| match t {
            capture::Target::Existing { id, label, part, kind, .. } => Some((id.clone(), label.clone(), *part, kind.clone())),
            capture::Target::New { .. } => None,
        })
        .collect();
    if !replaced.is_empty() && (cmd.chars().count() > MAX_COMMAND || cwd.chars().count() > MAX_CWD) {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            format!("a command that replaces a Vault value must fit in the approval prompt ({MAX_COMMAND} characters, folder {MAX_CWD}): put the steps in a script file and run that"),
        ));
    }
    let mut went_ahead = Vec::new();
    // Asked at once, each with its own deadline, so the asking fits the Terminal
    // tool's wait however many entries the command replaces.
    let who = &agent;
    let mut asking: FuturesUnordered<_> = replaced.iter().map(|(id, label, part, kind)| async move {
        let request = UseRequest {
            id: uuid::Uuid::new_v4().to_string(),
            agent: who.clone(),
            entry_id: id.clone(),
            entry_label: label.clone(),
            what: part.to_string(),
            origin: String::new(),
            field: String::new(),
            offer_always: false,
            reason: "what the command prints replaces the value".into(),
            agent_reason: reason.to_string(),
            kind: kind.clone(),
            command: cmd.to_string(),
            cwd: cwd.to_string(),
            replaces: true,
            requested_ms: agentmux_common::time::now_ms_u64(),
            ..Default::default()
        };
        (id, label, ask(state, agent_block_id, request, &format!("{who} wants to replace {label} with what a command prints")).await)
    }).collect();
    while let Some((id, label, ended)) = asking.next().await {
        if !ended.allowed() {
            vault::record_line(&state.identity_store, vault::Line { entry_id: id, label, actor: &agent, action: capture::CAPTURED, target: &clip(cmd, 160), outcome: ended.outcome(), reason });
            super::vault_handlers::publish(state);
            return Err(refused(ended));
        }
        if ended == Ended::WentAhead {
            went_ahead.push(label.clone());
        }
    }
    drop(asking);
    let changed_state = state.clone();
    let job = capture::Job { capture: plan, store: state.identity_store.clone(), changed: Box::new(move || super::vault_handlers::publish(&changed_state)) };
    Ok((Some(job), went_ahead))
}

/// One entry's question for a command: the entry, the parts of it asked for, the
/// request and its headline.
type Question<'a> = (&'a FillTarget, Vec<(&'a Delivery, &'a FillTarget)>, UseRequest, String);

/// Asks every question at once and keeps the "Always" grants. The labels of the
/// entries that went ahead with nobody answering, or the first that didn't go
/// ahead, and how: the rest are withdrawn then.
async fn ask_all<'a>(state: &AppState, agent_block_id: &str, agent: &str, questions: Vec<Question<'a>>) -> Result<Vec<String>, (&'a FillTarget, Ended)> {
    let mut asking: FuturesUnordered<_> = questions.into_iter().map(|(first, waiting_on, request, headline)| async move { (first, waiting_on, ask(state, agent_block_id, request, &headline).await) }).collect();
    let mut went_ahead = Vec::new();
    while let Some((first, waiting_on, ended)) = asking.next().await {
        match ended {
            Ended::Once => {}
            Ended::WentAhead => went_ahead.push(first.label.clone()),
            Ended::Always => {
                for (_, t) in &waiting_on {
                    let (store, id, key, created) = (state.identity_store.clone(), t.id.clone(), grant_key(agent, t.part), t.created_at);
                    let _ = tokio::task::spawn_blocking(move || store.vault_grant_add(&id, &key, agentmux_common::time::now_ms(), created)).await;
                }
            }
            ended => return Err((first, ended)),
        }
    }
    Ok(went_ahead)
}

/// A rotation's command, approved by the person with the rotation (`Vault`
/// `rotate`): exactly these secrets (`NEW_SECRET` naming the entry's pending
/// value, `OLD_SECRET` its current one). Only srv's rotation makes one, and hands
/// it to [`inject`] directly; nothing an agent sends can.
pub(super) struct Preapproved {
    pub env: std::collections::HashMap<String, String>,
}

/// The question for the person about running `command` (cmd, cwd) with the parts of `first`'s entry that
/// `waiting_on` names, and what the agent says it is for.
fn command_request(agent: &str, first: &FillTarget, waiting_on: &[(&Delivery, &FillTarget)], command: (&str, &str), agent_reason: &str) -> UseRequest {
    let mut whats: Vec<&str> = waiting_on.iter().map(|(_, t)| t.what.as_str()).collect();
    whats.dedup();
    UseRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.to_string(),
        entry_id: first.id.clone(),
        entry_label: first.label.clone(),
        what: whats.join(" and "),
        origin: String::new(),
        field: waiting_on.iter().map(|(d, t)| d.shown(t.part)).collect::<Vec<_>>().join(", "),
        offer_always: true,
        reason: format!("it is the first time {agent} runs a command with this entry"),
        agent_reason: agent_reason.to_string(),
        kind: first.kind.to_string(),
        command: command.0.to_string(),
        cwd: command.1.to_string(),
        requested_ms: agentmux_common::time::now_ms_u64(),
        ..Default::default()
    }
}

/// Resolves, asks for and reads the entries a command was given. `None` when it
/// was given none.
pub(super) async fn inject(
    state: &AppState,
    agent_block_id: &str,
    auth: Option<&agentmux_common::api_types::UiAutomationAuth>,
    wanted: Wanted<'_>,
    remote: bool,
    cmd: &str,
    cwd: &str,
    preapproved: Option<&Preapproved>,
) -> Result<Option<Injected>, Refusal> {
    let reason = wanted.reason.to_string();
    // A rotation's approval covers exactly the secrets it approved, nothing more.
    let preapproved = preapproved.is_some_and(|p| {
        wanted.stdin.is_none_or(|s| s.trim().is_empty()) && wanted.files.is_none_or(|f| f.is_empty()) && wanted.env == Some(&p.env)
    });
    let wanted = deliveries(wanted)?;
    if wanted.is_empty() {
        return Ok(None);
    }
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
    let agent = agent_of(state, agent_block_id, auth, "a Vault secret")?;
    let mut targets: Vec<(Delivery, FillTarget)> = Vec::new();
    for (delivery, entry) in wanted {
        let (store, who) = (state.identity_store.clone(), agent.clone());
        let target = tokio::task::spawn_blocking(move || vault::command_target(&store, &entry, Some(&who)))
            .await
            .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .map_err(|e| refuse(StatusCode::BAD_REQUEST, e))?;
        targets.push((delivery, target));
    }
    // Only a rotation's own command, approved with it, gets its pending value.
    if !preapproved && targets.iter().any(|(_, t)| t.version == Some("pending")) {
        return Err(refuse(StatusCode::BAD_REQUEST, "@pending is a rotation's new value: only that rotation's own commands get it"));
    }
    let record = |t: &FillTarget, outcome: &str| {
        vault::record_line(
            &state.identity_store,
            vault::Line { entry_id: &t.id, label: &t.label, actor: &agent, action: "ran a command", target: &clip(cmd, 160), outcome, reason: &reason },
        );
        // Every window's Vault pane shows the new line of the record.
        super::vault_handlers::publish(state);
    };

    // One question per entry, covering the parts of it the agent hasn't been allowed
    // to use in commands: a grant is for one part (a card's expiry is not its number).
    // They are asked at once, each with its own deadline (spec §7), so nobody
    // answering several waits 30 s for them together, within the Terminal tool's wait.
    let mut asked: Vec<&str> = Vec::new();
    let mut questions = Vec::new();
    for (_, first) in &targets {
        if preapproved {
            break;
        }
        if asked.contains(&first.id.as_str()) {
            continue;
        }
        asked.push(&first.id);
        let mut waiting_on: Vec<(&Delivery, &FillTarget)> = Vec::new();
        for (delivery, t) in targets.iter().filter(|(_, t)| t.id == first.id) {
            let (store, id, key) = (state.identity_store.clone(), t.id.clone(), grant_key(&agent, t.part));
            let granted = tokio::task::spawn_blocking(move || store.vault_grant_check(&id, &key).unwrap_or(false)).await.unwrap_or(false);
            if !granted {
                waiting_on.push((delivery, t));
            }
        }
        if waiting_on.is_empty() {
            continue;
        }
        let mut whats: Vec<&str> = waiting_on.iter().map(|(_, t)| t.what.as_str()).collect();
        whats.dedup();
        let what = whats.join(" and ");
        let request = command_request(&agent, first, &waiting_on, (cmd, cwd), &reason);
        questions.push((first, waiting_on, request, format!("{agent} wants to run a command with your {what}")));
    }
    let went_ahead = match ask_all(state, agent_block_id, &agent, questions).await {
        Ok(w) => w,
        Err((first, ended)) => {
            record(first, ended.outcome());
            return Err(refused(ended));
        }
    };

    // Allowed: the values leave their sealed rows now, into this process only.
    let mut env = Vec::new();
    let mut stdin = None;
    let mut files = Vec::new();
    let mut known: Vec<(String, String)> = Vec::new();
    for (delivery, target) in &targets {
        let (store, t) = (state.identity_store.clone(), target.clone());
        let value = match tokio::task::spawn_blocking(move || vault::value_for_command(&store, &t)).await {
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
        match delivery {
            Delivery::Env(name) => env.push((name.clone(), value.to_string())),
            Delivery::Stdin => stdin = Some(value),
            Delivery::File(name) => files.push((name.clone(), value)),
        }
    }
    for (_, target) in &targets {
        record(target, "done");
        let (store, id) = (state.identity_store.clone(), target.id.clone());
        let _ = tokio::task::spawn_blocking(move || store.vault_touch(&id, agentmux_common::time::now_ms())).await;
    }
    Ok(Some(Injected { env, stdin, files, scrubber: Scrubber::new(&known), went_ahead }))
}

#[cfg(test)]
#[path = "shell_secrets/tests.rs"]
mod tests;
