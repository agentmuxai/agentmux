// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/vault/import_batch`: an agent has the Vault copy a named list of secrets from AWS Secrets Manager
//! under one approval (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §4).
//!
//! The whole list is checked first. Nobody is asked: srv reads every row, under one AWS session, and writes a history line
//! for each (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md). It answers with a row per secret: its
//! entry's id, or why nothing was kept. Never a value. A batch counts one write per row against the agent's pace.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::vault_import::{asker, credentials, error, Who};
use super::AppState;
use crate::backend::vault::aws_import::{self, Via};
use crate::backend::vault::aws_import_batch::{self, Approved, BatchPlan, BatchRequest, ItemResult, BUDGET};
use crate::identity::aws_broker::mint::HostRunner;
use crate::identity::aws_broker::policy;

#[derive(Debug, Deserialize)]
pub(crate) struct BatchBody {
    #[serde(flatten)]
    request: BatchRequest,
    #[serde(default)]
    auth: Option<agentmux_common::api_types::UiAutomationAuth>,
}

/// Whether any row of `plan` is read from AWS: a row that is a link is not.
fn reads_aws(plan: &BatchPlan) -> bool {
    plan.items.iter().any(|p| !p.is_link())
}

/// The list checked, and for a container agent the grant it names, before anything is read.
async fn checked(state: &AppState, request: &BatchRequest, who: &Who) -> Result<BatchPlan, Response> {
    let (store, agent, req) = (state.identity_store.clone(), who.agent.clone(), request.clone());
    let plan = tokio::task::spawn_blocking(move || aws_import_batch::check(&store, &req, &agent))
        .await
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    // A link is made without a read: a container agent needs a grant only for a batch that copies something.
    if who.container && reads_aws(&plan) {
        let current = tokio::task::spawn_blocking(policy::current).await.unwrap_or_else(|e| Err(e.to_string()));
        aws_import::check_grant(&current, plan.profile.as_deref(), &who.slug).map_err(|e| error(StatusCode::FORBIDDEN, e))?;
    }
    Ok(plan)
}

/// What is said of a batch that did nothing. A list of links only copies nothing to begin with: nothing was recorded.
fn nothing(plan: &BatchPlan) -> &'static str {
    if plan.links_only() {
        "nothing was recorded"
    } else {
        "nothing was copied"
    }
}

/// Records what became of a batch that read nothing, and tells the agent.
fn refused(state: &AppState, plan: &BatchPlan, who: &Who, outcome: &str, status: StatusCode, said: String) -> Response {
    aws_import_batch::note(&state.identity_store, plan, &who.agent, outcome, &plan.asked_text());
    super::vault_handlers::publish(state);
    error(status, said)
}

/// What the agent is told of a batch that ran: a row per approved secret, and the rows the person left out.
fn answer(plan: &BatchPlan, approved: &Approved, results: &[ItemResult], outcome: &str) -> Value {
    let stored = results.iter().filter(|r| r.id.is_some()).count();
    json!({
        "status": outcome,
        "batch": plan.id,
        "name": plan.name,
        "vault": plan.vault_name,
        "list": approved.list,
        "stored": stored,
        "failed": results.len() - stored,
        "items": results,
        "not_approved": approved.left_out,
    })
}

/// Reads the approved rows under one AWS session (one broker mint for a container agent, whatever the rows), and
/// records the batch's own line.
async fn run(state: &AppState, plan: &BatchPlan, approved: &Approved, who: &Who) -> Response {
    // A link is made without a read, so a list of links only asks the broker for nothing.
    let minted = match approved.items.iter().any(|p| !p.is_link()) {
        true => match credentials(state, who, plan.profile.as_deref().unwrap_or_default(), &plan.reason).await {
            Ok(minted) => minted,
            Err(why) => return refused(state, plan, who, "failed", StatusCode::SERVICE_UNAVAILABLE, why),
        },
        false => None,
    };
    let via = minted.as_ref().map_or(Via::Host, Via::Session);
    let results = aws_import_batch::perform(&state.identity_store, &HostRunner, &approved.items, via, &who.agent, BUDGET).await;
    let (outcome, what) = plan.ran_text(approved, &results);
    aws_import_batch::note(&state.identity_store, plan, &who.agent, outcome, &what);
    super::vault_handlers::publish(state);
    Json(answer(plan, approved, &results, outcome)).into_response()
}

/// `POST /api/v1/vault/import_batch`
pub(crate) async fn handle_vault_import_batch(State(state): State<AppState>, caller: Option<axum::Extension<super::caller::Caller>>, Json(body): Json<BatchBody>) -> Response {
    let who = match asker(&state, caller.as_deref(), body.auth.as_ref(), "import_batch", "who copied each secret") {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };
    let plan = match checked(&state, &body.request, &who).await {
        Ok(plan) => plan,
        Err(refusal) => return refusal,
    };
    let acts: Vec<crate::backend::vault::pace::Act> = plan.items.iter().map(|p| super::vault_import::act_of(&p.target)).collect();
    if let Err(why) = crate::backend::vault::pace::admit(&who.agent, &acts) {
        return refused(&state, &plan, &who, "refused: pace", StatusCode::TOO_MANY_REQUESTS, why);
    }
    // Every row of the list, as the agent named it: nobody ticks rows.
    let every_row: Vec<usize> = (0..plan.items.len()).collect();
    let approved = plan.approved(&every_row);
    run(&state, &plan, &approved, &who).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::vault::aws_import::{Plan, Source, Target};

    fn item(secret: &str, key: Option<&str>, label: &str, replaces: bool) -> Plan {
        let target = if replaces {
            Target::Replace { id: "api-key-stripe".into(), created_at: 1, kind: "api-key", part: "value", label: label.into(), vault_id: "fleet".into() }
        } else {
            Target::New { kind: "login", part: "password", label: label.into(), fields: Default::default(), vault_id: "fleet".into() }
        };
        Plan {
            source: Source { aws_secret_id: secret.into(), key: key.map(String::from), user_key: None, region: Some("eu-west-1".into()), profile: Some("work-read".into()) },
            target,
            vault_name: "Fleet".into(),
            reason: "copy the build hosts' passwords for task 31007".into(),
            refs: vec!["batch:0123456789ab".into()],
            share: false,
        }
    }

    fn plan() -> BatchPlan {
        BatchPlan {
            id: "0123456789ab".into(),
            name: "host passwords".into(),
            vault_id: "fleet".into(),
            vault_name: "Fleet".into(),
            reason: "copy the build hosts' passwords for task 31007".into(),
            region: Some("eu-west-1".into()),
            profile: Some("work-read".into()),
            refs: vec!["batch:0123456789ab".into()],
            items: vec![item("hosts/alpha", Some("password"), "Alpha", false), item("pay/stripe", None, "Stripe", true)],
        }
    }

    #[test]
    fn a_batch_of_links_only_reads_nothing_in_aws_and_says_so() {
        // What a container agent's grant is asked for: a batch that copies anything. A link is made without a read.
        assert!(reads_aws(&plan()));
        let link = |label: &str| Plan { target: Target::Link { kind: "api-key", label: label.into(), fields: Default::default(), vault_id: "fleet".into() }, ..item("prod/mailer", None, label, false) };
        let links = BatchPlan { items: vec![link("Mail provider key"), link("Invoice provider key")], ..plan() };
        assert!(!reads_aws(&links));
        let mixed = BatchPlan { items: vec![link("Mail provider key"), item("hosts/alpha", Some("password"), "Alpha", false)], ..plan() };
        assert!(reads_aws(&mixed));
        // A list of links only copies nothing, so a refusal leaves nothing recorded; any other list, nothing copied.
        assert_eq!((nothing(&links), nothing(&plan())), ("nothing was recorded", "nothing was copied"));
    }

    #[test]
    fn a_batch_counts_a_write_a_row_for_the_pace_a_replacement_as_one_and_a_link_as_a_new_entry() {
        use crate::backend::vault::pace::Act;
        let acts: Vec<Act> = plan().items.iter().map(|p| super::super::vault_import::act_of(&p.target)).collect();
        assert_eq!(acts, [Act::New, Act::Replace("api-key-stripe".into())]);
        // Every row is read: nobody ticks rows, so the approval is the whole list.
        let every: Vec<usize> = (0..plan().items.len()).collect();
        assert_eq!(plan().approved(&every).items.len(), 2);
    }

    #[test]
    fn the_answer_is_a_row_per_approved_secret_and_the_rows_left_out() {
        let plan = plan();
        let approved = plan.approved(&[0]);
        let results = [ItemResult { label: "Alpha".into(), id: Some("login-alpha".into()), error: None }];
        let (outcome, _) = plan.ran_text(&approved, &results);
        let said = answer(&plan, &approved, &results, outcome);
        assert_eq!(said["items"], json!([{ "label": "Alpha", "id": "login-alpha" }]));
        assert_eq!((said["status"].as_str(), said["stored"].as_u64(), said["failed"].as_u64()), (Some("done"), Some(1), Some(0)));
        assert_eq!((said["not_approved"].clone(), said["vault"].as_str(), said["batch"].as_str()), (json!(["Stripe"]), Some("Fleet"), Some("0123456789ab")));
        assert_eq!(said["list"].as_str(), Some(approved.list.as_str()));
    }
}
