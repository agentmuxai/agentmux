// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/vault/import`: an agent has the Vault copy a secret from AWS Secrets Manager into an entry
//! (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §7).
//!
//! The arguments are checked first. Then the person is asked once (Allow once or Deny, never Always, and silence
//! denies), srv reads the secret with the host's AWS CLI, and only ids and names come back. A container agent reads
//! under a broker session for the grant it names; a native agent under the host's own AWS setup.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use crate::backend::vault::aws_import::{self, ImportRequest, Plan, Target, Via, SERVICE};
use crate::backend::vault::uses::{Ended, UseImport, UseRequest};
use crate::identity::aws_broker::mint::HostRunner;
use crate::identity::aws_broker::request::{credentials_for, Ask};
use crate::identity::aws_broker::{policy, sessions};

/// How long the person has to answer by default, and at most.
const ANSWER_WAIT: Duration = Duration::from_secs(120);
const ANSWER_WAIT_MAX: Duration = Duration::from_secs(600);

#[derive(Debug, Deserialize)]
pub(crate) struct ImportBody {
    #[serde(flatten)]
    request: ImportRequest,
    #[serde(default)]
    auth: Option<agentmux_common::api_types::UiAutomationAuth>,
    #[serde(default)]
    wait_secs: Option<u64>,
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// Who is asking, as the request's signed identity proves: the agent's id, its pane, its slug and whether it runs in a
/// container. An unproven request, or an agent AgentMux has no row for, is refused with the reason.
struct Who {
    agent: String,
    pane: String,
    slug: String,
    uid: String,
    container: bool,
}

fn asker(state: &AppState, body: &ImportBody) -> Result<Who, Response> {
    let refuse = |why: String| error(StatusCode::FORBIDDEN, format!("import needs the agent's signed identity: {why}"));
    let Some(auth) = body.auth.as_ref() else { return Err(refuse("the Vault records who copied each secret".into())) };
    let pane = super::ui_handlers::verified_block_id(state, None, auth).map_err(refuse)?;
    let agent: String = auth.agent_id.chars().take(64).collect();
    let uid = crate::backend::reactive::handler::get_global_handler().get_agent(&auth.agent_id).and_then(|r| r.uid);
    let def = uid.as_deref().and_then(|u| state.mstore.agent_def_get(u).ok().flatten());
    let Some(def) = def else { return Err(error(StatusCode::FORBIDDEN, "import couldn't tell which agent this is")) };
    Ok(Who { agent, pane, slug: def.slug.clone(), uid: uid.unwrap_or_default(), container: def.runtime().is_container() })
}

/// The question for the person: the secret, where it goes, and what the agent says it is for.
fn card(plan: &Plan, who: &Who) -> UseRequest {
    let replaces = matches!(plan.target, Target::Replace { .. });
    UseRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent: who.agent.clone(),
        block_id: who.pane.clone(),
        entry_id: plan.entry_id().to_string(),
        entry_label: plan.label().to_string(),
        what: "secret".into(),
        offer_always: false,
        reason: "an agent asked to copy a secret into your Vault".into(),
        agent_reason: plan.reason.clone(),
        kind: plan.kind().to_string(),
        requested_ms: agentmux_common::time::now_ms_u64(),
        import: Some(UseImport {
            service: SERVICE.into(),
            secret: plan.source.aws_secret_id.clone(),
            key: plan.source.key.clone().unwrap_or_default(),
            region: plan.source.region.clone().unwrap_or_default(),
            profile: plan.source.profile.clone().unwrap_or_default(),
            grant: who.container,
            vault: plan.vault_name.clone(),
            replaces,
        }),
        ..Default::default()
    }
}

fn headline(plan: &Plan, who: &Who) -> String {
    format!("{} wants to copy a secret from {SERVICE} into your Vault as {}", who.agent, plan.label())
}

/// What the agent is told when the card didn't end in a yes: silence is a no for an import, whatever the kind.
fn not_allowed(ended: Ended) -> String {
    match ended {
        Ended::Expired => "nobody answered, and a copy of a secret needs a yes: nothing was copied; ask the person".to_string(),
        other => other.refusal("nothing was copied"),
    }
}

/// The arguments checked, and for a container agent the grant it names, before anyone is asked.
async fn checked(state: &AppState, body: &ImportBody, who: &Who) -> Result<Plan, Response> {
    let (store, agent) = (state.identity_store.clone(), who.agent.clone());
    let req = body.request.clone();
    let plan = tokio::task::spawn_blocking(move || aws_import::check(&store, &req, &agent))
        .await
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    if who.container {
        let current = tokio::task::spawn_blocking(policy::current).await.unwrap_or_else(|e| Err(e.to_string()));
        aws_import::check_grant(&current, plan.source.profile.as_deref(), &who.slug).map_err(|e| error(StatusCode::FORBIDDEN, e))?;
    }
    Ok(plan)
}

/// Records what became of an import that stored nothing, and tells the agent.
fn refused(state: &AppState, plan: &Plan, who: &Who, outcome: &str, status: StatusCode, said: String) -> Response {
    aws_import::note(&state.identity_store, plan, plan.entry_id(), plan.label(), &who.agent, outcome);
    super::vault_handlers::publish(state);
    error(status, said)
}

/// How the CLI is given credentials: the host's own, or a session minted for the container agent's grant (the
/// broker's checks, hourly cap and Audit row apply as for any mint, with the import's reason).
async fn credentials(state: &AppState, plan: &Plan, who: &Who) -> Result<Option<crate::identity::aws_broker::mint::Minted>, String> {
    if !who.container {
        return Ok(None);
    }
    let current = tokio::task::spawn_blocking(policy::current).await.unwrap_or_else(|e| Err(e.to_string()));
    let ask = Ask { agent_uid: &who.uid, agent_slug: &who.slug, in_container: true, profile: plan.source.profile.as_deref().unwrap_or_default(), reason: Some(&plan.reason), seconds: None };
    match credentials_for(current, sessions::global(), &HostRunner, &ask, chrono::Utc::now()).await {
        Ok(granted) => {
            super::app_api::aws_credentials::audit_granted(state, &who.slug, &who.uid, &granted);
            Ok(Some(granted.creds))
        }
        Err(refusal) => {
            super::app_api::aws_credentials::audit_refused(state, &who.slug, &who.uid, ask.profile, &refusal);
            Err(refusal.text())
        }
    }
}

/// `POST /api/v1/vault/import`
pub(crate) async fn handle_vault_import(State(state): State<AppState>, Json(body): Json<ImportBody>) -> Response {
    let who = match asker(&state, &body) {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };
    let plan = match checked(&state, &body, &who).await {
        Ok(plan) => plan,
        Err(refusal) => return refusal,
    };
    let wait = body.wait_secs.map(Duration::from_secs).unwrap_or(ANSWER_WAIT).min(ANSWER_WAIT_MAX);
    let (ended, _) = super::vault_ask::ask(&state, &who.pane, card(&plan, &who), wait, &headline(&plan, &who)).await;
    if !ended.allowed() {
        return refused(&state, &plan, &who, ended.outcome(), StatusCode::FORBIDDEN, not_allowed(ended));
    }
    let minted = match credentials(&state, &plan, &who).await {
        Ok(minted) => minted,
        Err(why) => return refused(&state, &plan, &who, "failed", StatusCode::SERVICE_UNAVAILABLE, why),
    };
    let via = minted.as_ref().map_or(Via::Host, Via::Session);
    match aws_import::perform(&state.identity_store, &HostRunner, &plan, via, &who.agent).await {
        Ok(done) => {
            super::vault_handlers::publish(&state);
            Json(json!({ "status": "imported", "id": done.id, "label": done.label, "vault": done.vault })).into_response()
        }
        Err(why) => refused(&state, &plan, &who, "failed", StatusCode::BAD_GATEWAY, why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::vault::aws_import::{Source, Target};

    fn plan(replaces: bool) -> Plan {
        let target = if replaces {
            Target::Replace { id: "api-key-stripe".into(), created_at: 1, kind: "api-key", part: "value", label: "Stripe".into(), vault_id: "personal".into() }
        } else {
            Target::New { kind: "api-key", part: "value", label: "Staging database".into(), fields: Default::default(), vault_id: "personal".into() }
        };
        Plan {
            source: Source { aws_secret_id: "staging/db".into(), key: Some("db.password".into()), region: Some("eu-west-1".into()), profile: Some("work-read".into()) },
            target,
            vault_name: "Personal".into(),
            reason: "copy the staging database password for task 28071".into(),
            refs: vec![],
        }
    }

    fn who() -> Who {
        Who { agent: "builder".into(), pane: "block-1".into(), slug: "builder".into(), uid: "uid-1".into(), container: false }
    }

    #[test]
    fn the_card_names_the_secret_the_vault_and_what_the_agent_says_and_offers_no_always() {
        let in_container = super::card(&plan(false), &Who { container: true, ..who() });
        let card = card(&plan(false), &who());
        let import = card.import.as_ref().unwrap();
        assert_eq!((import.service.as_str(), import.secret.as_str(), import.key.as_str(), import.region.as_str(), import.vault.as_str()), (SERVICE, "staging/db", "db.password", "eu-west-1", "Personal"));
        assert!(!import.replaces && !card.offer_always && !card.needs_value);
        assert_eq!((import.profile.as_str(), import.grant), ("work-read", false), "the card says which AWS login the read runs under");
        assert!(in_container.import.unwrap().grant, "a container agent's profile is a grant");
        assert_eq!((card.agent_reason.as_str(), card.block_id.as_str(), card.entry_id.as_str()), ("copy the staging database password for task 28071", "block-1", ""));
    }

    #[test]
    fn silence_on_an_import_says_it_needed_a_yes_not_that_an_id_or_card_did() {
        let said = not_allowed(Ended::Expired);
        assert!(said.contains("needs a yes") && said.contains("nothing was copied") && !said.contains("ID or card"), "{said}");
        assert_eq!(not_allowed(Ended::Denied), "the person said no: nothing was copied");
    }

    #[test]
    fn a_replacement_card_says_which_entry_and_silence_never_lets_either_go_ahead() {
        let replacing = card(&plan(true), &who());
        assert!(replacing.import.as_ref().unwrap().replaces);
        assert_eq!((replacing.entry_id.as_str(), replacing.entry_label.as_str()), ("api-key-stripe", "Stripe"));
        // An api-key use goes ahead on silence; an import of one never does.
        assert!(crate::backend::vault::uses::goes_ahead_on_silence("api-key", false));
        for plan in [plan(false), plan(true)] {
            assert!(!crate::backend::vault::uses::goes_ahead(&card(&plan, &who())));
        }
    }
}
