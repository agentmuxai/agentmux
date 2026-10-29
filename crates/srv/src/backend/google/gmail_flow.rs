// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! One `Gmail` write call end to end (G3): whose account and whether the agent may write
//! ([`super::access`]), whether the token covers the action, the person's yes for what can't be taken
//! back (`send_draft`, `trash_thread`: SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §6),
//! the call, and a line in the account's log however it ends.
//!
//! The yes comes from [`Approver`], which in srv is the host's approval window: a window an agent
//! can neither click nor forge. A send is checked twice: the draft is read and shown, and read
//! again after the yes, and a draft that changed in between is not sent.

use std::future::Future;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::backend::storage::google_grants::GrantLevel;
use crate::backend::storage::store::AgentDefinition;
use crate::identity::oauth_tokens::{access_token_for, HttpReply, OAuthHttp, TokenSecrets};

use super::access::{access_for, Access};
use super::calls::{log_call, CallDeps};
use super::gmail::{self, GmailArgs};
use super::gmail_write::{self as write, DraftView};
use super::{google_error, GoogleHttp, Service};

/// What the person answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    /// The window closed, timed out or could not be opened.
    Unanswered,
}

/// Asks the person a yes or no about what `def` wants to do.
pub trait Approver: Sync {
    fn ask(&self, def: &AgentDefinition, title: String, message: String, yes: &str, no: &str) -> impl Future<Output = Result<Answer, String>> + Send;
}

/// Sends an agent can complete in a day, and questions it can put in an hour: a backstop in case a
/// tricked agent asks over and over (§6).
const SENDS_PER_DAY: usize = 20;
const ASKS_PER_HOUR: usize = 10;

/// A `Gmail` write action for agent `def`.
pub async fn gmail_write<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets, A: Approver>(
    deps: &CallDeps<'_, G, O, S>,
    approver: &A,
    def: &AgentDefinition,
    action: &str,
    args: &GmailArgs,
    account: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    if !write::WRITE_ACTIONS.contains(&action) {
        return Err(format!("Gmail has no write action \"{action}\"; one of {}", write::WRITE_ACTIONS.join(", ")));
    }
    let access = access_for(deps.mstore, deps.id_store, deps.identity_store, def, Service::Gmail, account)?;
    let ids = logged_ids(action, args);
    let result = write_with(deps, approver, &access, def, action, args, now).await;
    log_call(deps, &access, def, Service::Gmail, action, ids, &result, now);
    result.map(|r| json!({ "account": access.email(), "result": r }))
}

/// The ids a write touched, for the account's log (D7): never the mail.
fn logged_ids(action: &str, args: &GmailArgs) -> String {
    let t = |v: &Option<String>| v.as_deref().map(str::trim).unwrap_or("").to_string();
    match action {
        "label_thread" | "trash_thread" => t(&args.thread_id),
        _ => t(&args.draft_id),
    }
}

async fn write_with<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets, A: Approver>(
    deps: &CallDeps<'_, G, O, S>,
    approver: &A,
    access: &Access,
    def: &AgentDefinition,
    action: &str,
    args: &GmailArgs,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    if access.level < GrantLevel::Write {
        return Err(format!(
            "{} lets you read Gmail but not write it. The person can allow drafts, labels and sending for you in Connectors > Accounts > {} > Gmail",
            access.email(),
            access.email()
        ));
    }
    if !write::granted_for(action, &access.granted_scopes()) {
        return Err(format!(
            "{} hasn't granted the Gmail permission {action} needs. The person can turn Gmail writes on again for the account in Connectors",
            access.email()
        ));
    }
    // Plans are built before anything is asked or sent, so a bad argument costs the person nothing.
    match action {
        "create_draft" | "update_draft" => {
            let req = write::draft_plan(action, args)?;
            let reply = send(deps, access, &req).await?;
            Ok(shape_draft(&reply.body))
        }
        "label_thread" => {
            let req = write::label_plan(args)?;
            let reply = send(deps, access, &req).await?;
            Ok(json!({ "thread_id": reply.body.get("id").and_then(Value::as_str).unwrap_or(""), "done": true }))
        }
        "send_draft" => send_draft(deps, approver, access, def, args, now).await,
        "trash_thread" => trash_thread(deps, approver, access, def, args, now).await,
        _ => unreachable!("checked against WRITE_ACTIONS"),
    }
}

/// One request to Google with the account's token (refreshed in srv); a refusal is a sentence.
async fn send<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets>(
    deps: &CallDeps<'_, G, O, S>,
    access: &Access,
    req: &super::GoogleRequest,
) -> Result<HttpReply, String> {
    let token = access_token_for(deps.oauth, deps.secrets, deps.id_store, deps.identity_store, &access.account, None).await?;
    let reply = deps.google.send(req, &token).await?;
    if (200..300).contains(&reply.status) {
        Ok(reply)
    } else {
        Err(error_of(&reply))
    }
}

fn error_of(reply: &HttpReply) -> String {
    match reply.status {
        404 => "Google found no such draft or thread for this account (the id may be from another account, or it was deleted)".to_string(),
        403 if reply.body.pointer("/error/message").and_then(Value::as_str).is_some_and(|m| m.contains("has not been used") || m.contains("is disabled")) => {
            "the Gmail API isn't turned on in the Google Cloud project of this account's OAuth client: the person can enable it in the Google Cloud console (APIs & Services)".to_string()
        }
        _ => google_error(reply),
    }
}

fn shape_draft(body: &Value) -> Value {
    json!({
        "draft_id": body.get("id").and_then(Value::as_str).unwrap_or(""),
        "thread_id": body.pointer("/message/threadId").and_then(Value::as_str).unwrap_or(""),
        "note": "The draft is in the person's Gmail Drafts. Sending it is send_draft, which asks the person first.",
    })
}

/// How many questions (`send_draft`, `trash_thread`) this agent has put on this account in the last
/// hour, and how many mails it has sent in the last day, from the account's log.
fn recent(deps: &CallDeps<'_, impl GoogleHttp, impl OAuthHttp, impl TokenSecrets>, access: &Access, def: &AgentDefinition, now: DateTime<Utc>) -> (usize, usize) {
    let calls = deps.identity_store.google_calls(&access.account.id, 500).unwrap_or_default();
    let ms = now.timestamp_millis();
    let mine = |c: &&crate::backend::storage::google_grants::GoogleCall| c.agent_id == def.id && c.service == "gmail";
    let asks = calls.iter().filter(mine).filter(|c| write::always_asks(&c.action) && c.at > ms - 3_600_000).count();
    let sends = calls.iter().filter(mine).filter(|c| c.action == "send_draft" && c.outcome == "ok" && c.at > ms - 86_400_000).count();
    (asks, sends)
}

async fn ask<A: Approver>(approver: &A, def: &AgentDefinition, (title, message): (String, String), yes: &str, no: &str, what: &str) -> Result<(), String> {
    match approver.ask(def, title, message, yes, no).await? {
        Answer::Yes => Ok(()),
        Answer::No => Err(format!("the person said no, so {what} was not done")),
        Answer::Unanswered => Err(format!("the person didn't answer, so {what} was not done. Don't ask again straight away; tell them in your reply what you wanted to do")),
    }
}

async fn send_draft<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets, A: Approver>(
    deps: &CallDeps<'_, G, O, S>,
    approver: &A,
    access: &Access,
    def: &AgentDefinition,
    args: &GmailArgs,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    let fetch = write::draft_fetch_plan(args)?;
    let (asks, sends) = recent(deps, access, def, now);
    if sends >= SENDS_PER_DAY {
        return Err(format!("you have sent {SENDS_PER_DAY} mails from this account today, the most an agent may: wait until tomorrow"));
    }
    if asks >= ASKS_PER_HOUR {
        return Err(format!("you have asked the person {ASKS_PER_HOUR} times in the last hour to send or trash mail: wait before asking again"));
    }
    let view = DraftView::of(&send(deps, access, &fetch).await?.body);
    if !view.has_recipient() {
        return Err("the draft has no recipient: update_draft it with to first".to_string());
    }
    let question = write::send_question(&def.name, &access.email(), &view);
    ask(approver, def, question, "Send", "Don't send", "sending").await?;
    // What the person said yes to is what goes: read the draft again and compare.
    let again = DraftView::of(&send(deps, access, &fetch).await?.body);
    if again.digest() != view.digest() {
        return Err("the draft changed after the person approved it, so it was not sent. Call send_draft again to ask about the new text".to_string());
    }
    let reply = send(deps, access, &write::send_plan(&view.id)?).await?;
    Ok(json!({
        "sent": true,
        "message_id": reply.body.get("id").and_then(Value::as_str).unwrap_or(""),
        "thread_id": reply.body.get("threadId").and_then(Value::as_str).unwrap_or(""),
    }))
}

async fn trash_thread<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets, A: Approver>(
    deps: &CallDeps<'_, G, O, S>,
    approver: &A,
    access: &Access,
    def: &AgentDefinition,
    args: &GmailArgs,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    let trash = write::trash_plan(args)?;
    let summary = gmail::summary_plan(args.thread_id.as_deref().unwrap_or(""))?;
    let (asks, _) = recent(deps, access, def, now);
    if asks >= ASKS_PER_HOUR {
        return Err(format!("you have asked the person {ASKS_PER_HOUR} times in the last hour to send or trash mail: wait before asking again"));
    }
    let headers = send(deps, access, &summary).await?.body;
    let question = write::trash_question(&def.name, &access.email(), &headers);
    ask(approver, def, question, "Move to trash", "Keep it", "trashing").await?;
    let reply = send(deps, access, &trash).await?;
    Ok(json!({ "thread_id": reply.body.get("id").and_then(Value::as_str).unwrap_or(""), "trashed": true }))
}

#[cfg(test)]
#[path = "gmail_flow_tests.rs"]
mod tests;
