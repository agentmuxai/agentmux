// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Filling a saved sign-in by the person's own choice
//! (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.3).
//!
//! When a frame of a pane shows a sign-in form, the host says so
//! (`credential.Form`); srv lists the `login` entries the tab's profile holds for
//! exactly that frame's origin in the block meta [`SIGNINS_META_KEY`] (entry id,
//! origin, masked user name: never a password). The pane shows a key button with
//! their number, which opens a menu marked operator-only, so no agent's `UI`
//! click reaches its items: the person choosing "Sign in as al***" is their
//! approval. That sends `browser.signin_fill`, which only the window may, and
//! srv hands the host the user name and password to type into that frame.
//!
//! Only `https`, or `http` on this computer: a sign-in is never typed into a
//! page anyone on the network could read or change.

use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::backend::vault::signins;

use super::AppState;

/// The block meta key the pane reads its saved sign-ins from.
pub(crate) const SIGNINS_META_KEY: &str = "browser:signins";
const HOST_WAIT: Duration = Duration::from_secs(20);

/// The frame origins each pane's page showed a sign-in form for.
static FORMS: LazyLock<Mutex<HashMap<String, BTreeSet<String>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Whether a sign-in may be typed into a page of `origin`: `https`, or `http` on this computer.
pub(crate) fn fillable(origin: &str) -> bool {
    if origin.starts_with("https://") {
        return true;
    }
    let Some(host) = origin.strip_prefix("http://") else { return false };
    let host = host.rsplit_once(':').map_or(host, |(h, port)| if port.chars().all(|c| c.is_ascii_digit()) { h } else { host });
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// A frame of pane `block_id` of `origin` shows a sign-in form.
pub(crate) fn form(state: &AppState, block_id: &str, origin: &str) {
    if !fillable(origin) {
        return;
    }
    let added = FORMS.lock().entry(block_id.to_string()).or_default().insert(origin.to_string());
    if added {
        publish(state, block_id);
    }
}

/// Forget every pane's forms and clear the lists saved with them. Called when
/// the host registers: after srv restarts the forms are gone while a list saved
/// with a pane stays, and after the host restarts its pages are new and it no
/// longer knows to say they navigated. Either way a page reports its forms again.
pub(crate) fn reset_lists(state: &AppState) {
    FORMS.lock().clear();
    super::browser_signin_offer::clear_stale(state, SIGNINS_META_KEY, &BTreeSet::new(), "[signin-fill]");
}

/// Pane `block_id`'s main frame loaded a new page: its forms went with the old one.
pub(crate) fn navigated(state: &AppState, block_id: &str) {
    if FORMS.lock().remove(block_id).is_some() {
        set_meta(state, block_id, Value::Null);
    }
}

/// The saved sign-ins pane `block_id`'s page can take, as the pane lists them.
fn listed(state: &AppState, block_id: &str) -> Vec<Value> {
    let origins: Vec<String> = FORMS.lock().get(block_id).map(|o| o.iter().cloned().collect()).unwrap_or_default();
    let Some(profile) = super::browser_signin_offer::profile_of(state, block_id) else { return Vec::new() };
    match signins::offered_for(&state.identity_store, &profile, &origins) {
        Ok(found) => found.into_iter().map(|(entry, origin, user)| json!({ "entry": entry, "origin": origin, "user": signins::mask(&user) })).collect(),
        Err(e) => {
            tracing::warn!(error = %e, "[signin-fill] couldn't read the saved sign-ins");
            Vec::new()
        }
    }
}

/// Show pane `block_id` the saved sign-ins its page can take (none: no button).
pub(crate) fn publish(state: &AppState, block_id: &str) {
    if !FORMS.lock().contains_key(block_id) {
        return;
    }
    let list = listed(state, block_id);
    set_meta(state, block_id, if list.is_empty() { Value::Null } else { Value::Array(list) });
}

fn set_meta(state: &AppState, block_id: &str, value: Value) {
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(SIGNINS_META_KEY.to_string(), value);
    if let Err(e) = super::http_shell::broadcast_meta_update(state, block_id, &meta) {
        tracing::warn!(error = %e, "[signin-fill] couldn't show the pane's saved sign-ins");
    }
}

/// The person chose entry `entry` from pane `block_id`'s "Sign in as" menu: type
/// it into the page, if the page still shows a form for the site it is saved for.
pub(crate) async fn fill(state: &AppState, block_id: &str, entry: &str) -> Result<(), String> {
    let profile = super::browser_signin_offer::profile_of(state, block_id).ok_or("this tab keeps no sign-ins (Incognito)")?;
    let origins: Vec<String> = FORMS.lock().get(block_id).map(|o| o.iter().cloned().collect()).unwrap_or_default();
    let (store, id) = (state.identity_store.clone(), entry.to_string());
    let (origin, username, password, label) =
        tokio::task::spawn_blocking(move || signins::for_fill(&store, &id, &profile, &origins)).await.map_err(|e| e.to_string())??;
    if !fillable(&origin) {
        return Err(format!("{origin} isn't https, so the sign-in wasn't typed"));
    }
    let body = json!({ "block_id": block_id, "origin": origin, "username": username, "password": password.as_str() });
    let typed = super::ui_handlers::proxy_value(state, "signin_fill", body, HOST_WAIT).await;
    drop(password);
    let outcome = if typed.is_ok() { "done" } else { "failed" };
    crate::backend::vault::record(&state.identity_store, entry, &label, crate::backend::vault::PERSON, signins::SIGNED_IN, &origin, outcome);
    if typed.is_ok() {
        if let Err(e) = state.identity_store.vault_touch(entry, agentmux_common::time::now_ms()) {
            tracing::warn!(error = %e, "[signin-fill] couldn't note the sign-in's use");
        }
        super::vault_handlers::publish(state);
    }
    typed.map(|_| ()).map_err(|(_, e)| e)
}

/// `browser.signin_fill`, from the window only: an agent's sign-in goes through
/// `Browser` `signin` and its approval window (B3), never this.
pub(crate) fn register(engine: &std::sync::Arc<crate::backend::rpc::engine::WshRpcEngine>, state: &AppState) {
    use crate::backend::rpc_types::{CommandBrowserSigninFillData, COMMAND_BROWSER_SIGNIN_FILL};
    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_SIGNIN_FILL, move |req: CommandBrowserSigninFillData, ctx| {
        let state = st.clone();
        async move {
            ctx.operator_only("only the person chooses a saved sign-in to fill")?;
            fill(&state, &req.block_id, &req.entry).await
        }
    });
}

#[cfg(test)]
#[path = "browser_signin_fill_tests.rs"]
mod tests;
