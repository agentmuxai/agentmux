// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! One agent tool call to Google, end to end: whose account and grant
//! ([`super::access`]), whether the token covers the action, a fresh token
//! (srv refreshes it; the agent never sees it), the request, the trimmed
//! answer, and a line in the account's log (D7) however it ends.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::backend::storage::google_grants::{GoogleCall, GrantLevel};
use crate::backend::storage::store::{AgentDefinition, Store};
use crate::identity::oauth_tokens::{access_token_for, HttpReply, OAuthHttp, TokenSecrets};

use super::access::{access_for, Access};
use super::calendar::{self, CalendarArgs};
use super::drive::{self, DriveArgs};
use super::gmail::{self, GmailArgs};
use super::{google_error, GoogleHttp, Service};

/// What a call needs from srv.
pub struct CallDeps<'a, G: GoogleHttp, O: OAuthHttp, S: TokenSecrets> {
    pub google: &'a G,
    pub oauth: &'a O,
    pub secrets: &'a S,
    pub mstore: &'a Store,
    pub id_store: &'a Arc<Store>,
    pub identity_store: &'a Arc<Store>,
}

/// A `Calendar` read action for agent `def`. The answer names the account
/// and carries the trimmed result.
pub async fn calendar_read<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets>(
    deps: &CallDeps<'_, G, O, S>,
    def: &AgentDefinition,
    action: &str,
    args: &CalendarArgs,
    account: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    if !calendar::READ_ACTIONS.contains(&action) {
        return Err(format!("Calendar has no action \"{action}\"; one of {}", calendar::READ_ACTIONS.join(", ")));
    }
    let access = access_for(deps.mstore, deps.id_store, deps.identity_store, def, Service::Calendar, account)?;
    let ids = calendar::logged_ids(action, args);
    let result = calendar_read_with(deps, &access, action, args, now).await;
    log_call(deps, &access, def, Service::Calendar, action, ids, &result, now);
    result.map(|r| json!({ "account": access.email(), "result": r }))
}

/// A line in the account's log (D7) for one call, however it ended.
#[allow(clippy::too_many_arguments)]
pub(crate) fn log_call<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets>(
    deps: &CallDeps<'_, G, O, S>,
    access: &Access,
    def: &AgentDefinition,
    service: Service,
    action: &str,
    ids: String,
    result: &Result<Value, String>,
    now: DateTime<Utc>,
) {
    let outcome = match result {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("error: {}", super::clip(e, 200)),
    };
    let line = GoogleCall {
        at: now.timestamp_millis(),
        agent_id: def.id.clone(),
        agent_name: def.name.clone(),
        service: service.as_str().to_string(),
        action: action.to_string(),
        ids,
        outcome,
    };
    if let Err(e) = deps.identity_store.google_call_log(&access.account.id, &line) {
        tracing::warn!(target: "google", account_id = %access.account.id, error = %e, "google: couldn't write the call log");
    }
}

async fn calendar_read_with<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets>(
    deps: &CallDeps<'_, G, O, S>,
    access: &Access,
    action: &str,
    args: &CalendarArgs,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    debug_assert!(access.level >= GrantLevel::Read);
    if !calendar::granted_for(action, &access.granted_scopes()) {
        return Err(format!(
            "{} hasn't granted the Calendar permission {action} needs (it was only partly granted). The person can turn Calendar on again for the account in Connectors",
            access.email()
        ));
    }
    let req = calendar::plan(action, args, now)?;
    let token = access_token_for(deps.oauth, deps.secrets, deps.id_store, deps.identity_store, &access.account, None).await?;
    let reply = deps.google.send(&req, &token).await?;
    if !(200..300).contains(&reply.status) {
        return Err(google_error(&reply));
    }
    Ok(calendar::shape(action, &reply.body))
}

/// A `Drive` read action for agent `def`. `pick_files` is not one of them (the handler asks
/// the person); everything else is a request to Google and a trimmed answer.
pub async fn drive_read<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets>(
    deps: &CallDeps<'_, G, O, S>,
    def: &AgentDefinition,
    action: &str,
    args: &DriveArgs,
    account: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    if !drive::READ_ACTIONS.contains(&action) {
        return Err(format!("Drive has no action \"{action}\"; one of {}, pick_files", drive::READ_ACTIONS.join(", ")));
    }
    let access = access_for(deps.mstore, deps.id_store, deps.identity_store, def, Service::Drive, account)?;
    let ids = drive::logged_ids(action, args);
    let result = drive_read_with(deps, &access, action, args).await;
    log_call(deps, &access, def, Service::Drive, action, ids, &result, now);
    result.map(|r| json!({ "account": access.email(), "result": r }))
}

async fn drive_read_with<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets>(
    deps: &CallDeps<'_, G, O, S>,
    access: &Access,
    action: &str,
    args: &DriveArgs,
) -> Result<Value, String> {
    debug_assert!(access.level >= GrantLevel::Read);
    if !drive::granted_for(action, &access.granted_scopes()) {
        return Err(format!(
            "{} hasn't granted the Drive permission {action} needs. The person can turn Drive on again for the account in Connectors",
            access.email()
        ));
    }
    let req = drive::plan(action, args)?;
    let token = access_token_for(deps.oauth, deps.secrets, deps.id_store, deps.identity_store, &access.account, None).await?;
    let reply = deps.google.send(&req, &token).await?;
    if !(200..300).contains(&reply.status) {
        return Err(drive_error(&reply, action));
    }
    if action != "read_file_content" {
        return Ok(drive::shape(action, &reply.body));
    }
    let meta = reply.body;
    let content_req = drive::content_plan(&meta)?;
    let content = deps.google.send(&content_req, &token).await?;
    if !(200..300).contains(&content.status) {
        return Err(drive_error(&content, action));
    }
    Ok(drive::shape_content(&meta, &content.body))
}

/// A `Gmail` read action for agent `def`.
pub async fn gmail_read<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets>(
    deps: &CallDeps<'_, G, O, S>,
    def: &AgentDefinition,
    action: &str,
    args: &GmailArgs,
    account: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    if !gmail::READ_ACTIONS.contains(&action) {
        return Err(format!("Gmail has no action \"{action}\"; one of {}", gmail::READ_ACTIONS.join(", ")));
    }
    let access = access_for(deps.mstore, deps.id_store, deps.identity_store, def, Service::Gmail, account)?;
    let ids = gmail::logged_ids(action, args);
    let result = gmail_read_with(deps, &access, action, args).await;
    log_call(deps, &access, def, Service::Gmail, action, ids, &result, now);
    result.map(|r| json!({ "account": access.email(), "result": r }))
}

async fn gmail_read_with<G: GoogleHttp, O: OAuthHttp, S: TokenSecrets>(
    deps: &CallDeps<'_, G, O, S>,
    access: &Access,
    action: &str,
    args: &GmailArgs,
) -> Result<Value, String> {
    debug_assert!(access.level >= GrantLevel::Read);
    if !gmail::granted_for(action, &access.granted_scopes()) {
        return Err(format!(
            "{} hasn't granted the Gmail permission {action} needs. The person can turn Gmail on again for the account in Connectors",
            access.email()
        ));
    }
    let req = gmail::plan(action, args)?;
    let token = access_token_for(deps.oauth, deps.secrets, deps.id_store, deps.identity_store, &access.account, None).await?;
    let reply = deps.google.send(&req, &token).await?;
    if !(200..300).contains(&reply.status) {
        return Err(gmail_error(&reply));
    }
    if action == "get_thread" {
        // A long mail's text can be stored outside the message: fetch those parts (not files) first.
        let mut thread = reply.body;
        load_external_text(deps.google, &token, &mut thread).await;
        return Ok(gmail::shape(action, &thread));
    }
    if action != "search_threads" {
        return Ok(gmail::shape(action, &reply.body));
    }
    // Google's search lists ids and snippets; the first few threads get their headers, one
    // request each. A thread whose headers can't be had is listed without them.
    let mut summaries = Vec::new();
    let threads = reply.body.get("threads").and_then(Value::as_array).cloned().unwrap_or_default();
    for t in threads.iter().take(gmail::SUMMARIZED) {
        let id = t.get("id").and_then(Value::as_str).unwrap_or("");
        let one = match gmail::summary_plan(id) {
            Ok(r) => deps.google.send(&r, &token).await.ok().filter(|r| (200..300).contains(&r.status)).map(|r| r.body),
            Err(_) => None,
        };
        summaries.push(one.unwrap_or(Value::Null));
    }
    Ok(gmail::shape_search(&reply.body, &summaries))
}

/// Fetch the text parts `thread` (`{ messages: [..] }`) keeps outside its messages and put them in,
/// as far as Google gives them. Files are never fetched.
pub(super) async fn load_external_text<G: GoogleHttp>(google: &G, token: &str, thread: &mut Value) {
    let mut parts = std::collections::HashMap::new();
    for (mid, aid) in gmail::external_text_parts(thread) {
        let Ok(r) = gmail::part_plan(&mid, &aid) else { continue };
        let Some(body) = google.send(&r, token).await.ok().filter(|r| (200..300).contains(&r.status)).map(|r| r.body) else { continue };
        match body.get("data").and_then(Value::as_str) {
            Some(d) if d.len() <= gmail::EXTERNAL_PART_CHARS => {
                parts.insert(aid, d.to_string());
            }
            _ => {}
        }
    }
    gmail::fill_external(thread, &parts);
}

/// Google's refusal of a Gmail call. A 404 is a thread that isn't there.
fn gmail_error(reply: &HttpReply) -> String {
    match reply.status {
        404 => "Google found no such thread for this account (the id may be from another account, or the thread was deleted)".to_string(),
        403 if reply.body.pointer("/error/message").and_then(Value::as_str).is_some_and(|m| m.contains("has not been used") || m.contains("is disabled")) => {
            "the Gmail API isn't turned on in the Google Cloud project of this account's OAuth client: the person can enable it in the Google Cloud console (APIs & Services)".to_string()
        }
        _ => google_error(reply),
    }
}

/// Google's refusal of a Drive call. A 404 on `drive.file` is usually a file the person
/// hasn't picked, which the generic sentence doesn't say.
fn drive_error(reply: &HttpReply, action: &str) -> String {
    if reply.status == 404 && action != "search_files" {
        return "Google found no such file for this account. With the permission AgentMux asks for, an agent reaches only the files the person picked (Drive pick_files asks them) and the ones AgentMux made".to_string();
    }
    google_error(reply)
}

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;
