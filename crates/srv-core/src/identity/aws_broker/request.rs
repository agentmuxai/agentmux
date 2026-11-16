// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One request for credentials, from the checks to the answer (spec §5), with the agent already identified.
//! Everything that can be decided without an HTTP server or a real process is decided here, so it is tested without
//! either.

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{DateTime, Utc};

use super::mint::{self, clean_reason, MintCtx, MintError, Minted, Runner};
use super::policy::{Grant, Policy};
use super::sessions::{SessionError, Sessions, Source};

/// A refusal: the status it travels with, its code, and the sentence the agent is shown (spec §10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }

    /// `CODE: sentence`, what the agent's SDK prints.
    pub fn text(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}

/// Who is asking and for what. The agent is the one the container's token was minted for.
pub struct Ask<'a> {
    pub agent_uid: &'a str,
    pub agent_slug: &'a str,
    pub in_container: bool,
    pub profile: &'a str,
    pub reason: Option<&'a str>,
    pub seconds: Option<u64>,
}

pub struct Granted {
    pub creds: Minted,
    pub grant: Grant,
    pub source: Source,
    /// The reason as it was cleaned and used.
    pub reason: String,
    /// The lifetime asked for, within the grant's bounds.
    pub seconds: u64,
}

fn list(names: &[&str]) -> String {
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

/// The checks of spec §5 in order, then the cache or the mint.
pub async fn credentials_for<R: Runner>(
    policy: Result<Option<Arc<Policy>>, String>,
    sessions: &Sessions,
    runner: &R,
    ask: &Ask<'_>,
    now: DateTime<Utc>,
) -> Result<Granted, Refusal> {
    if !ask.in_container {
        return Err(Refusal::new(
            StatusCode::FORBIDDEN,
            "NATIVE_AGENT",
            "This is for container agents. A native agent has the host's own AWS setup.",
        ));
    }
    let policy = match policy {
        Ok(Some(policy)) if !policy.grants.is_empty() => policy,
        Ok(_) => {
            return Err(Refusal::new(
                StatusCode::FORBIDDEN,
                "AWS_NOT_SHARED",
                "AWS isn't shared with container agents on this machine. A person adds grants in ~/.agentmux/aws-grants.json \
                 (the AgentMux docs have a page on it); until then there is no AWS from here.",
            ));
        }
        Err(e) => {
            return Err(Refusal::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "AWS_POLICY_UNREADABLE",
                format!("The grants file on this machine doesn't parse, so AWS can't be shared until a person fixes it: {e}"),
            ));
        }
    };
    let Some(grant) = policy.grants.get(ask.profile) else {
        return Err(Refusal::new(
            StatusCode::NOT_FOUND,
            "UNKNOWN_PROFILE",
            format!("No AWS grant named '{}'. This machine shares: {}.", ask.profile, list(&policy.names())),
        ));
    };
    if grant.disabled {
        let why = grant.note.as_deref().map(|n| format!(" {n}")).unwrap_or_default();
        return Err(Refusal::new(StatusCode::FORBIDDEN, "GRANT_DISABLED", format!("The grant '{}' is switched off on this machine.{why}", grant.name)));
    }
    if !grant.open_to(ask.agent_slug) {
        let open: Vec<&str> = grant.agents.iter().map(String::as_str).collect();
        return Err(Refusal::new(
            StatusCode::FORBIDDEN,
            "NOT_ALLOWED_FOR_YOU",
            format!(
                "The grant '{}' isn't open to you. It is open to: {}. Ask the person, or an agent that can edit the policy, to add you.",
                grant.name,
                list(&open)
            ),
        ));
    }
    let reason = clean_reason(ask.reason.unwrap_or_default());
    if grant.needs_reason && reason.is_empty() {
        return Err(Refusal::new(
            StatusCode::BAD_REQUEST,
            "REASON_REQUIRED",
            format!("The grant '{}' needs a reason. Run the command with AGENTMUX_AWS_REASON='what and why' in front of it.", grant.name),
        ));
    }
    let seconds = ask.seconds.unwrap_or(grant.default_seconds).clamp(super::policy::MIN_SECONDS, grant.max_seconds);
    let ctx = MintCtx { agent_slug: ask.agent_slug, grant, reason: &reason, seconds, now_ms: now.timestamp_millis() };
    let got = sessions
        .get_or_mint(ask.agent_uid, &grant.name, &reason, grant.max_mints_per_hour, now, || mint::mint(runner, &ctx, now))
        .await;
    match got {
        Ok((creds, source)) => Ok(Granted { creds, grant: grant.clone(), source, reason, seconds }),
        Err(SessionError::TooMany { cap }) => Err(Refusal::new(
            StatusCode::TOO_MANY_REQUESTS,
            "TOO_MANY_MINTS",
            format!(
                "You asked for '{}' credentials {cap} times in the last hour. The job may be looping; wait, or ask for a higher cap.",
                grant.name
            ),
        )),
        Err(SessionError::Mint(e)) => Err(match e {
            MintError::CliMissing(why) => Refusal::new(StatusCode::SERVICE_UNAVAILABLE, "AWS_CLI_MISSING", format!("{why}.")),
            MintError::LoginNeeded(said) => {
                let profile = match &grant.mint {
                    super::policy::Mint::HostProfile { profile } => profile.clone(),
                    super::policy::Mint::Command { .. } => grant.name.clone(),
                };
                Refusal::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "AWS_HOST_LOGIN_NEEDED",
                    format!("The host's AWS login for '{profile}' has lapsed. A person runs `aws sso login --profile {profile}` on this computer. ({said})"),
                )
            }
            MintError::Refused(said) => Refusal::new(StatusCode::SERVICE_UNAVAILABLE, "AWS_HOST_REFUSED", format!("The host's AWS refused: {said}")),
            MintError::TookTooLong => {
                Refusal::new(StatusCode::SERVICE_UNAVAILABLE, "AWS_TOOK_TOO_LONG", "AWS credentials took too long (25 s). Try again in a minute.")
            }
        }),
    }
}

#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
