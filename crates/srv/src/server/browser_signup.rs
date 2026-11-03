// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Use a strong password" on a sign-up form
//! (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §6).
//!
//! When the person focuses a field for a new password in a pane's page, the
//! host says so (`credential.Signup`, with the field's `passwordrules`,
//! `minlength` and `maxlength` and the user name typed so far); srv offers it in
//! the block meta [`SIGNUP_META_KEY`] (the frame's origin only), and the pane's
//! key button gets "Use a strong password (saved to the Vault)" in its menu,
//! marked operator-only. Choosing it sends `browser.signup_password`, which only
//! the window may: srv generates a password under the field's rules, keeps it
//! as a `login` entry tagged unconfirmed **before** anything is typed, so the
//! value exists only in the Vault when the site first sees it, then has the host
//! set it into the new-password field and its confirm field.
//!
//! When the form is sent with it (`credential.SignupSent`), srv records the user
//! name sent and drops the tag. A form not sent within 30 minutes leaves the
//! entry unconfirmed in the Vault. Choosing it again (the site refused the
//! password) replaces the unconfirmed value, the refused one kept as a version.
//! A sign-up sent with a password the person typed is offered "Save to Vault?"
//! like a sign-in.

use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::backend::vault::generate::{self, RulesRequest};
use crate::backend::vault::signins::Space;
use crate::backend::vault::{signups, PERSON};

use super::AppState;

/// The block meta key the pane reads the offer from.
pub(crate) const SIGNUP_META_KEY: &str = "browser:signup";
/// How long a sent form can still confirm the password made for it.
const CONFIRM_WITHIN: Duration = Duration::from_secs(30 * 60);
const HOST_WAIT: Duration = Duration::from_secs(20);

/// The new-password field a pane's page last had the focus in: its frame's
/// origin, the token the listener marked it with, and what the page says a
/// password there may be.
#[derive(Debug, Clone, Default)]
pub(crate) struct Field {
    pub origin: String,
    /// Only this field is filled: `data-agentmux-signup` holds it.
    pub field: String,
    pub username: String,
    pub passwordrules: Option<String>,
    pub minlength: Option<usize>,
    pub maxlength: Option<usize>,
}

/// The entry last made for a pane's sign-up, until the form is sent with it.
#[derive(Debug, Clone)]
struct Made {
    origin: String,
    entry: String,
    at: Instant,
}

static OFFERED: LazyLock<Mutex<HashMap<String, Field>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static MADE: LazyLock<Mutex<HashMap<String, Made>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The person focused a new-password field of `field.origin` in pane `block_id`.
/// Nothing is offered in Incognito, or on a page anyone on the network could
/// read or change.
pub(crate) fn focused(state: &AppState, block_id: &str, field: Field) {
    if !token(&field.field) || !super::browser_signin_fill::fillable(&field.origin) || super::browser_signin_offer::profile_of(state, block_id).is_none() {
        return;
    }
    let origin = field.origin.clone();
    let was = OFFERED.lock().insert(block_id.to_string(), field).map(|f| f.origin);
    if was.as_deref() != Some(origin.as_str()) {
        set_meta(state, block_id, json!({ "origin": origin }));
    }
}

/// A field token as the listener makes one: letters, digits and `-`, at most 64.
fn token(t: &str) -> bool {
    !t.is_empty() && t.len() <= 64 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Pane `block_id`'s main frame loaded a new page: its fields went with the old one.
pub(crate) fn navigated(state: &AppState, block_id: &str) {
    if OFFERED.lock().remove(block_id).is_some() {
        set_meta(state, block_id, Value::Null);
    }
}

/// Forget every pane's offer and clear the ones saved with panes. Called when
/// the host registers, as `browser_signin_fill::reset_lists` is; the page offers
/// again the next time the person focuses the field.
pub(crate) fn reset_offers(state: &AppState) {
    OFFERED.lock().clear();
    super::browser_signin_offer::clear_stale(state, SIGNUP_META_KEY, &BTreeSet::new(), "[signup]");
}

fn set_meta(state: &AppState, block_id: &str, value: Value) {
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(SIGNUP_META_KEY.to_string(), value);
    if let Err(e) = super::http_shell::broadcast_meta_update(state, block_id, &meta) {
        tracing::warn!(error = %e, "[signup] couldn't show the pane's offer");
    }
}

/// A password for `field`: under its `passwordrules`, else its `minlength` and
/// `maxlength`, else the defaults, clipped to `maxlength`. Rules that can't be
/// read are dropped (the length limits still hold); rules that can are the
/// site's, so when they leave no strong password, or contradict each other,
/// none is made rather than one the site would refuse.
pub(crate) fn new_password(field: &Field) -> Result<Zeroizing<String>, String> {
    let length = field.minlength.filter(|n| *n > generate::DEFAULT_LENGTH).map(|n| n.min(generate::MAX_LENGTH));
    let passwordrules = field.passwordrules.clone().filter(|r| !r.trim().is_empty() && generate::readable(r));
    let rules = generate::rules(&RulesRequest { passwordrules, length, words: None, maxlength: field.maxlength })
        .map_err(|e| format!("this field's rules leave no strong password, so none was made: {e}"))?;
    generate::generate(&rules)
}

/// The person chose "Use a strong password" in pane `block_id`: make one, keep
/// it in the Vault, then set it into the page. Gives the site.
pub(crate) async fn make(state: &AppState, block_id: &str) -> Result<String, String> {
    let profile = super::browser_signin_offer::profile_of(state, block_id).ok_or("this tab keeps no sign-ins (Incognito)")?;
    let field = OFFERED.lock().get(block_id).cloned().ok_or("the page has no field for a new password now")?;
    let earlier = MADE.lock().get(block_id).filter(|m| m.origin == field.origin && m.at.elapsed() < CONFIRM_WITHIN).map(|m| m.entry.clone());
    let password = new_password(&field)?;
    let (origin, field_token) = (field.origin.clone(), field.field.clone());
    let space = Space { profile, origin: origin.clone(), realm: String::new(), proxy: false };
    let (store, value) = (state.identity_store.clone(), password.clone());
    let (entry, label) = tokio::task::spawn_blocking(move || {
        if let Some(id) = earlier {
            if let Some(label) = signups::replace(&store, &id, &value)? {
                return Ok((id, label));
            }
        }
        signups::create(&store, &space, &field.username, &value)
    })
    .await
    .map_err(|e| e.to_string())??;
    MADE.lock().insert(block_id.to_string(), Made { origin: origin.clone(), entry: entry.clone(), at: Instant::now() });
    super::vault_handlers::publish(state);
    let body = json!({ "block_id": block_id, "origin": origin, "field": field_token, "password": password.as_str() });
    let filled = super::ui_handlers::proxy_value(state, "signup_fill", body, HOST_WAIT).await;
    drop(password);
    let outcome = if filled.is_ok() { "done" } else { "failed" };
    crate::backend::vault::record(&state.identity_store, &entry, &label, PERSON, "filled on sign-up", &origin, outcome);
    filled.map_err(|(_, e)| format!("the password is saved in the Vault as {label}, but wasn't filled: {e}"))?;
    Ok(origin)
}

/// A sign-up form of `origin` was sent in pane `block_id`. The password made
/// for it, sent as made, confirms its entry; any other password is the
/// person's own, offered "Save to Vault?" as a sign-in is.
pub(crate) fn sent(state: &AppState, block_id: &str, origin: &str, username: Zeroizing<String>, password: Zeroizing<String>) {
    let made = MADE.lock().get(block_id).filter(|m| m.origin == origin && m.at.elapsed() < CONFIRM_WITHIN).cloned();
    if let Some(m) = made {
        match signups::confirm(&state.identity_store, &m.entry, &username, &password) {
            Ok(true) => {
                let mut made = MADE.lock();
                if made.get(block_id).is_some_and(|now| now.entry == m.entry) {
                    made.remove(block_id);
                }
                drop(made);
                super::vault_handlers::publish(state);
                return;
            }
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "[signup] couldn't confirm the sign-up's password"),
        }
    }
    super::browser_signin_offer::offer(state, block_id, origin, username, Some(password));
}

/// `browser.signup_password`, from the window only: an agent can't make the
/// person's password for them through the pane's menu.
pub(crate) fn register(engine: &std::sync::Arc<crate::backend::rpc::engine::WshRpcEngine>, state: &AppState) {
    use crate::backend::rpc_types::{CommandBrowserSignupPasswordData, COMMAND_BROWSER_SIGNUP_PASSWORD};
    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_SIGNUP_PASSWORD, move |req: CommandBrowserSignupPasswordData, ctx| {
        let state = st.clone();
        async move {
            ctx.operator_only("only the person chooses to make a password for a sign-up")?;
            make(&state, &req.block_id).await.map(|_| ())
        }
    });
}

#[cfg(test)]
#[path = "browser_signup_tests.rs"]
mod tests;
