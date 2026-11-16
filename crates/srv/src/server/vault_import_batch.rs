// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/vault/import_batch`: an agent has the Vault copy a named list of secrets from AWS Secrets Manager
//! under one approval (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §4).
//!
//! The whole list is checked first. Then the person is asked once, with a row for every secret and a tick on each
//! (Allow or Deny, never Always, and silence denies). srv reads the rows left ticked, and no other, under one AWS
//! session, and answers with a row per secret: its entry's id, or why nothing was kept. Never a value.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::vault_import::{asker, credentials, error, not_allowed, Who, ANSWER_WAIT, ANSWER_WAIT_MAX};
use super::AppState;
use crate::backend::vault::aws_import::{self, Target, Via, SERVICE};
use crate::backend::vault::aws_import_batch::{self, secrets, Approved, BatchPlan, BatchRequest, ItemResult, BUDGET};
use crate::backend::vault::uses::{Decision, UseBatch, UseBatchItem, UseBatchVault, UseRequest};
use crate::backend::vault::vaults::{self, VaultAgents};
use crate::identity::aws_broker::mint::HostRunner;
use crate::identity::aws_broker::policy;

#[derive(Debug, Deserialize)]
pub(crate) struct BatchBody {
    #[serde(flatten)]
    request: BatchRequest,
    #[serde(default)]
    auth: Option<agentmux_common::api_types::UiAutomationAuth>,
    #[serde(default)]
    wait_secs: Option<u64>,
}

/// The question for the person: the list's name, the one account, region and Vault, a row for every secret, and what
/// the agent says it is for.
fn card(plan: &BatchPlan, who: &Who, into: &Into) -> UseRequest {
    let items = plan
        .items
        .iter()
        .map(|p| UseBatchItem {
            label: p.label().to_string(),
            kind: p.kind().to_string(),
            secret: p.source.aws_secret_id.clone(),
            key: p.source.key.clone().unwrap_or_default(),
            replaces: matches!(p.target, Target::Replace { .. }),
            link: p.is_link(),
        })
        .collect();
    UseRequest {
        id: uuid::Uuid::new_v4().to_string(),
        agent: who.agent.clone(),
        block_id: who.pane.clone(),
        entry_label: plan.name.clone(),
        what: "secrets".into(),
        offer_always: false,
        reason: "an agent asked to copy secrets into your Vault".into(),
        agent_reason: plan.reason.clone(),
        requested_ms: agentmux_common::time::now_ms_u64(),
        batch: Some(UseBatch {
            name: plan.name.clone(),
            service: SERVICE.into(),
            region: plan.region.clone().unwrap_or_default(),
            profile: plan.profile.clone().unwrap_or_default(),
            grant: who.container,
            vault: plan.vault_name.clone(),
            agents: into.agents.clone(),
            vaults: into.vaults.clone(),
            items,
        }),
        ..Default::default()
    }
}

/// Whether any row of `plan` is read from AWS: a row that is a link is not.
fn reads_aws(plan: &BatchPlan) -> bool {
    plan.items.iter().any(|p| !p.is_link())
}

fn headline(plan: &BatchPlan, who: &Who) -> String {
    if !reads_aws(plan) {
        let keys = if plan.items.len() == 1 { "1 key lives".to_string() } else { format!("{} keys live", plan.items.len()) };
        return format!("{} wants to record where {keys}: {}", who.agent, plan.name);
    }
    format!("{} wants to copy {} from {SERVICE} into your Vault: {}", who.agent, secrets(plan.items.len()), plan.name)
}

/// The Vault the person chose for a batch's rows, as their answer names it: one the card offered. `None` for any
/// other use, for a no, and when they left the Vault as the card had it.
pub(super) fn chosen_vault(request: &UseRequest, vault: Option<&str>, decision: Decision) -> Result<Option<String>, String> {
    let (Some(batch), Some(vault)) = (request.batch.as_ref(), vault.map(str::trim).filter(|v| !v.is_empty())) else { return Ok(None) };
    if decision == Decision::Deny || vault == batch.vault {
        return Ok(None);
    }
    if !batch.vaults.iter().any(|v| v.name == vault) {
        return Err("that Vault isn't one the card offers".into());
    }
    Ok(Some(vault.to_string()))
}

/// The rows of a batch the person left ticked, as their answer names them: places in the request's own list, and at
/// least one for a yes. `None` for any other use, and for a no.
pub(super) fn ticked_rows(request: &UseRequest, rows: Option<&Value>, decision: Decision) -> Result<Option<Vec<usize>>, String> {
    let Some(batch) = request.batch.as_ref() else { return Ok(None) };
    if decision == Decision::Deny {
        return Ok(None);
    }
    let rows: Vec<usize> = rows.cloned().map(serde_json::from_value).transpose().map_err(|_| "rows is the places of the ticked rows in the list".to_string())?.unwrap_or_default();
    if rows.is_empty() {
        return Err("tick at least one row, or deny".into());
    }
    if rows.iter().any(|row| *row >= batch.items.len()) {
        return Err("that row isn't in the list".into());
    }
    Ok(Some(rows))
}

/// The list checked, and for a container agent the grant it names, before anyone is asked.
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

/// What the card says of the Vault the rows go into: who may use what is in it, and the Vaults the person may choose
/// instead.
#[derive(Debug, Clone, Default)]
struct Into {
    agents: VaultAgents,
    vaults: Vec<UseBatchVault>,
}

async fn vault_lines(state: &AppState, plan: &BatchPlan, who: &Who) -> Into {
    let (store, agent, plan) = (state.identity_store.clone(), who.agent.clone(), plan.clone());
    tokio::task::spawn_blocking(move || {
        let agents = store.vault_info(&plan.vault_id).ok().flatten().map(|row| vaults::agents_of(&row)).unwrap_or_default();
        let offered = aws_import_batch::choices(&store, &plan, &agent);
        Into { agents, vaults: offered.iter().map(|row| UseBatchVault { name: row.name.clone(), agents: vaults::agents_of(row) }).collect() }
    })
    .await
    .unwrap_or_default()
}

/// `plan`'s batch, checked again for the Vault the person chose; why not, when it can't go there.
async fn rechecked(state: &AppState, request: &BatchRequest, who: &Who, chosen: String, plan: &BatchPlan) -> Result<BatchPlan, String> {
    let (store, agent, req, plan, name) = (state.identity_store.clone(), who.agent.clone(), request.clone(), plan.clone(), chosen.clone());
    tokio::task::spawn_blocking(move || aws_import_batch::check_into(&store, &req, &agent, &chosen, &plan))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|why| format!("the person chose the Vault {name}, and the batch can't go there: {why}; nothing was copied"))
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
pub(crate) async fn handle_vault_import_batch(State(state): State<AppState>, Json(body): Json<BatchBody>) -> Response {
    let who = match asker(&state, body.auth.as_ref(), "import_batch", "who copied each secret") {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };
    let plan = match checked(&state, &body.request, &who).await {
        Ok(plan) => plan,
        Err(refusal) => return refusal,
    };
    let wait = body.wait_secs.map(Duration::from_secs).unwrap_or(ANSWER_WAIT).min(ANSWER_WAIT_MAX);
    let into = vault_lines(&state, &plan, &who).await;
    let (ended, said) = super::vault_ask::ask(&state, &who.pane, card(&plan, &who, &into), wait, &headline(&plan, &who)).await;
    if !ended.allowed() {
        return refused(&state, &plan, &who, ended.outcome(), StatusCode::FORBIDDEN, not_allowed(ended));
    }
    // The person may have sent the rows to another Vault: the batch is checked again for it, as the same batch.
    let plan = match said.as_ref().and_then(|a| a.vault.clone()) {
        None => plan,
        Some(chosen) => match rechecked(&state, &body.request, &who, chosen, &plan).await {
            Ok(moved) => moved,
            Err(why) => return refused(&state, &plan, &who, "failed", StatusCode::BAD_REQUEST, why),
        },
    };
    let approved = plan.approved(said.as_ref().and_then(|a| a.rows.as_deref()).unwrap_or_default());
    if approved.items.is_empty() {
        return refused(&state, &plan, &who, "denied", StatusCode::FORBIDDEN, "the person approved no row: nothing was copied".into());
    }
    run(&state, &plan, &approved, &who).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::vault::aws_import::{Plan, Source};
    use crate::backend::vault::uses;

    fn item(secret: &str, key: Option<&str>, label: &str, replaces: bool) -> Plan {
        let target = if replaces {
            Target::Replace { id: "api-key-stripe".into(), created_at: 1, kind: "api-key", part: "value", label: label.into(), vault_id: "fleet".into() }
        } else {
            Target::New { kind: "login", part: "password", label: label.into(), fields: Default::default(), vault_id: "fleet".into() }
        };
        Plan {
            source: Source { aws_secret_id: secret.into(), key: key.map(String::from), region: Some("eu-west-1".into()), profile: Some("work-read".into()) },
            target,
            vault_name: "Fleet".into(),
            reason: "copy the build hosts' passwords for task 31007".into(),
            refs: vec!["batch:0123456789ab".into()],
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

    fn who() -> Who {
        Who { agent: "builder".into(), pane: "block-1".into(), slug: "builder".into(), uid: "uid-1".into(), container: false }
    }

    #[test]
    fn the_card_names_the_list_the_account_the_vault_and_every_row_and_never_goes_ahead_on_silence() {
        let card = card(&plan(), &who(), &Into::default());
        let batch = card.batch.as_ref().unwrap();
        assert_eq!((batch.name.as_str(), batch.service.as_str(), batch.region.as_str(), batch.profile.as_str(), batch.grant, batch.vault.as_str()), ("host passwords", SERVICE, "eu-west-1", "work-read", false, "Fleet"));
        let rows: Vec<_> = batch.items.iter().map(|i| (i.label.as_str(), i.kind.as_str(), i.secret.as_str(), i.key.as_str(), i.replaces)).collect();
        assert_eq!(rows, [("Alpha", "login", "hosts/alpha", "password", false), ("Stripe", "api-key", "pay/stripe", "", true)]);
        assert_eq!((card.agent_reason.as_str(), card.block_id.as_str(), card.entry_label.as_str()), ("copy the build hosts' passwords for task 31007", "block-1", "host passwords"));
        assert!(!card.offer_always && !card.needs_value && card.import.is_none());
        assert!(!uses::goes_ahead(&card), "a batch of api keys waits for a yes, though a use of one api key goes ahead on silence");
        assert!(super::card(&plan(), &Who { container: true, ..who() }, &Into::default()).batch.unwrap().grant, "a container agent's profile is a grant");
        assert_eq!(headline(&plan(), &who()), "builder wants to copy 2 secrets from AWS Secrets Manager into your Vault: host passwords");
    }

    #[test]
    fn a_batch_of_links_only_reads_nothing_in_aws_and_says_so() {
        // What a container agent's grant is asked for: a batch that copies anything. A link is made without a read.
        assert!(reads_aws(&plan()));
        let link = |label: &str| Plan { target: Target::Link { kind: "api-key", label: label.into(), fields: Default::default(), vault_id: "fleet".into() }, ..item("prod/mailer", None, label, false) };
        let links = BatchPlan { items: vec![link("Mail provider key"), link("Invoice provider key")], ..plan() };
        assert!(!reads_aws(&links));
        assert_eq!(headline(&links, &who()), "builder wants to record where 2 keys live: host passwords");
        let mixed = BatchPlan { items: vec![link("Mail provider key"), item("hosts/alpha", Some("password"), "Alpha", false)], ..plan() };
        assert!(reads_aws(&mixed));
        assert!(headline(&mixed, &who()).starts_with("builder wants to copy 2 secrets from"));
    }

    #[test]
    fn the_card_says_who_may_use_the_vault_and_a_choice_is_one_it_offers() {
        let named = VaultAgents::Named(vec!["builder".into()]);
        let into = Into { agents: named.clone(), vaults: vec![UseBatchVault { name: "Fleet".into(), agents: named.clone() }, UseBatchVault { name: "Personal".into(), agents: VaultAgents::All }] };
        let card = card(&plan(), &who(), &into);
        let batch = card.batch.as_ref().unwrap();
        assert_eq!((batch.vault.as_str(), &batch.agents, batch.vaults.len()), ("Fleet", &named, 2));
        // Leaving the Vault as asked, or saying no, chooses nothing; another Vault must be one the card offers.
        assert_eq!(chosen_vault(&card, Some("Fleet"), Decision::Once), Ok(None));
        assert_eq!(chosen_vault(&card, None, Decision::Once), Ok(None));
        assert_eq!(chosen_vault(&card, Some("Personal"), Decision::Deny), Ok(None));
        assert_eq!(chosen_vault(&card, Some(" Personal "), Decision::Once), Ok(Some("Personal".to_string())));
        assert_eq!(chosen_vault(&card, Some("Closed"), Decision::Once), Err("that Vault isn't one the card offers".to_string()));
        // No other use takes a Vault.
        assert_eq!(chosen_vault(&UseRequest::default(), Some("Personal"), Decision::Once), Ok(None));
    }

    #[test]
    fn a_yes_names_the_ticked_rows_of_the_list_and_nothing_else() {
        let card = card(&plan(), &who(), &Into::default());
        assert_eq!(ticked_rows(&card, Some(&json!([1, 0])), Decision::Once), Ok(Some(vec![1, 0])));
        assert_eq!(ticked_rows(&card, Some(&json!([0])), Decision::Once), Ok(Some(vec![0])));
        for none in [None, Some(json!([]))] {
            assert_eq!(ticked_rows(&card, none.as_ref(), Decision::Once), Err("tick at least one row, or deny".to_string()));
        }
        assert_eq!(ticked_rows(&card, Some(&json!([0, 2])), Decision::Once), Err("that row isn't in the list".to_string()));
        for bad in [json!("all"), json!([-1]), json!(["0"]), json!([0.5])] {
            assert!(ticked_rows(&card, Some(&bad), Decision::Once).unwrap_err().starts_with("rows is"), "{bad}");
        }
        assert_eq!(ticked_rows(&card, Some(&json!([0, 2])), Decision::Deny), Ok(None), "a no needs no rows");
        let other = UseRequest { id: "u1".into(), ..Default::default() };
        assert_eq!(ticked_rows(&other, Some(&json!([0])), Decision::Once), Ok(None), "any other use has no rows");
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
