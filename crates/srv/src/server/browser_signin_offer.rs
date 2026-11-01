// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! "Save to Vault?" for a sign-in a browser pane saw
//! (SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.1, §5.2).
//!
//! The host hands srv the user name and password a page sent (`credential.Offer`);
//! srv holds them here, in memory, for 2 minutes, never logged. When the page
//! moves on (`credential.Settled`: a navigation, or the password field left the
//! page) the pane shows the offer from its block meta [`OFFER_META_KEY`], which
//! holds only the exact origin and the masked user name, never page text. The
//! person's answer (`browser.signin_offer_answer`, from the window only) saves it
//! into the tab's profile, remembers "never for this site", or drops it. The
//! offer goes after 2 minutes or the next navigation, and the held pair with it.
//!
//! Incognito never saves (§9.1). A step that asks only for the user name is held
//! the same 2 minutes and joined to the password step that follows.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::backend::vault::signins::{self, Space};

use super::AppState;

/// The block meta key the pane reads the offer from.
pub(crate) const OFFER_META_KEY: &str = "browser:signin_offer";
/// How long a sign-in is held for the person's answer.
const HOLD: Duration = Duration::from_secs(120);
/// A navigation this soon after the offer showed is the same sign-in finishing
/// (a redirect, or the field leaving and then the page moving), not the person
/// moving on.
const SETTLING: Duration = Duration::from_secs(5);
/// One offer per site in this long: a page luring clicks can't stack them up.
const PER_SITE: Duration = Duration::from_secs(15);

struct Held {
    id: String,
    profile: String,
    origin: String,
    username: Zeroizing<String>,
    password: Zeroizing<String>,
    update: bool,
    at: Instant,
    shown_at: Option<Instant>,
}

struct Identifier {
    origin: String,
    username: Zeroizing<String>,
    at: Instant,
}

#[derive(Default)]
struct Offers {
    /// The sign-in each pane holds: one at a time.
    held: HashMap<String, Held>,
    /// A user name sent alone, by pane, for the password step.
    identifiers: HashMap<String, Identifier>,
    /// When each site last showed an offer, by profile and origin.
    shown: HashMap<(String, String), Instant>,
}

static OFFERS: LazyLock<Mutex<Offers>> = LazyLock::new(|| Mutex::new(Offers::default()));

/// What the person answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    Save,
    Never,
    NotNow,
}

impl Answer {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "save" => Some(Self::Save),
            "never" => Some(Self::Never),
            "not_now" => Some(Self::NotNow),
            _ => None,
        }
    }
}

/// The profile a pane keeps sign-ins under; `None` for Incognito or a pane that is gone.
fn profile_of(state: &AppState, block_id: &str) -> Option<String> {
    let block = state.mstore.get::<crate::backend::obj::Block>(block_id).ok().flatten()?;
    signins::profile_for_tab(block.meta.get(super::browser_identity::IDENTITY_META_KEY).and_then(Value::as_str))
}

/// The host saw a sign-in sent in pane `block_id`'s frame of `origin`: hold it
/// until the page moves on. `password` is `None` for a step that asks only for
/// the user name.
pub(crate) fn offer(state: &AppState, block_id: &str, origin: &str, username: Zeroizing<String>, password: Option<Zeroizing<String>>) {
    let Some(profile) = profile_of(state, block_id) else { return };
    let mut offers = OFFERS.lock();
    let now = Instant::now();
    // What was held for panes that never moved on goes with time.
    offers.held.retain(|_, h| h.at.elapsed() < HOLD);
    offers.identifiers.retain(|_, i| i.at.elapsed() < HOLD);
    let Some(password) = password.filter(|p| !p.is_empty()) else {
        if !username.is_empty() {
            offers.identifiers.insert(block_id.to_string(), Identifier { origin: origin.to_string(), username, at: now });
        }
        return;
    };
    let joined = match offers.identifiers.remove(block_id) {
        Some(i) if username.is_empty() && i.origin == origin && i.at.elapsed() < HOLD => i.username,
        _ => username,
    };
    let space = Space { profile: profile.clone(), origin: origin.to_string(), realm: String::new(), proxy: false };
    let update = match decide(state, &space, &joined, &password) {
        Some(update) => update,
        None => return,
    };
    offers.held.insert(
        block_id.to_string(),
        Held { id: uuid::Uuid::new_v4().to_string(), profile, origin: origin.to_string(), username: joined, password, update, at: now, shown_at: None },
    );
}

/// Whether to offer saving `username` and `password` for `space`: `None` when
/// the person said never for the site or the Vault already holds exactly that;
/// `Some(true)` when it holds another password for that user (an update).
fn decide(state: &AppState, space: &Space, username: &str, password: &str) -> Option<bool> {
    let store = &state.identity_store;
    match store.browser_signin_never_has(&space.profile, &space.origin) {
        Ok(false) => {}
        Ok(true) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "[signin-offer] couldn't read the never-save list");
            return None;
        }
    }
    match signins::saved_as(store, space, username, password) {
        Ok(None) => Some(false),
        Ok(Some(same)) => (!same).then_some(true),
        Err(e) => {
            tracing::warn!(error = %e, "[signin-offer] couldn't read the saved sign-ins");
            None
        }
    }
}

/// Pane `block_id`'s page moved on (a navigation, or the password field left the
/// page): a held sign-in's offer shows; one already showing goes, unless the
/// sign-in is still finishing.
pub(crate) fn settled(state: &AppState, block_id: &str) {
    enum Then {
        Nothing,
        /// Dropped before it showed (a site that just showed one).
        Drop,
        Clear,
        Show(String, Value, Duration),
    }
    let then = {
        let mut guard = OFFERS.lock();
        let o = &mut *guard;
        let Some(held) = o.held.get_mut(block_id) else { return };
        let then = match held.shown_at {
            _ if held.at.elapsed() >= HOLD => Then::Clear,
            Some(at) if at.elapsed() < SETTLING => Then::Nothing,
            Some(_) => Then::Clear,
            None => {
                let site = (held.profile.clone(), held.origin.clone());
                o.shown.retain(|_, at| at.elapsed() < PER_SITE);
                if o.shown.contains_key(&site) {
                    Then::Drop
                } else {
                    held.shown_at = Some(Instant::now());
                    o.shown.insert(site, Instant::now());
                    Then::Show(held.id.clone(), banner(held), HOLD.saturating_sub(held.at.elapsed()))
                }
            }
        };
        if matches!(then, Then::Drop | Then::Clear) {
            o.held.remove(block_id);
        }
        then
    };
    match then {
        Then::Nothing | Then::Drop => {}
        Then::Clear => clear(state, block_id),
        Then::Show(id, banner, left) => {
            show(state, block_id, banner);
            expire_later(state.clone(), block_id.to_string(), id, left);
        }
    }
}

/// What the pane shows: the exact origin and the masked user name, and whether
/// it replaces a saved password. Never the password, never page text.
fn banner(held: &Held) -> Value {
    json!({
        "id": held.id,
        "origin": held.origin,
        "user": if held.username.is_empty() { String::new() } else { signins::mask(&held.username) },
        "update": held.update,
    })
}

fn show(state: &AppState, block_id: &str, banner: Value) {
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(OFFER_META_KEY.to_string(), banner);
    if let Err(e) = super::http_shell::broadcast_meta_update(state, block_id, &meta) {
        tracing::warn!(error = %e, "[signin-offer] couldn't show the offer");
    }
}

fn clear(state: &AppState, block_id: &str) {
    show(state, block_id, Value::Null);
}

/// Take offer `id` down after `after`, if it is still up.
fn expire_later(state: AppState, block_id: String, id: String, after: Duration) {
    tokio::spawn(async move {
        tokio::time::sleep(after).await;
        let gone = {
            let mut offers = OFFERS.lock();
            let mine = offers.held.get(&block_id).is_some_and(|h| h.id == id);
            if mine {
                offers.held.remove(&block_id);
            }
            mine
        };
        if gone {
            clear(&state, &block_id);
        }
    });
}

/// The person answered offer `id` in pane `block_id`.
pub(crate) fn answer(state: &AppState, block_id: &str, id: &str, answer: Answer) -> Result<(), String> {
    let held = {
        let mut offers = OFFERS.lock();
        match offers.held.get(block_id) {
            Some(h) if h.id == id => offers.held.remove(block_id),
            _ => None,
        }
    };
    clear(state, block_id);
    let Some(held) = held else { return Err("that offer has gone: the sign-in wasn't kept".to_string()) };
    let store = &state.identity_store;
    match answer {
        Answer::Save => {
            let space = Space { profile: held.profile, origin: held.origin, realm: String::new(), proxy: false };
            signins::save(store, &space, &held.username, &held.password)?;
            super::vault_handlers::publish(state);
        }
        Answer::Never => store.browser_signin_never_add(&held.profile, &held.origin, agentmux_common::time::now_ms()).map_err(|e| e.to_string())?,
        Answer::NotNow => {}
    }
    Ok(())
}

/// `browser.signin_offer_answer`, from the window only: an agent can't save,
/// refuse or drop the person's sign-in for them.
pub(crate) fn register(engine: &std::sync::Arc<crate::backend::rpc::engine::WshRpcEngine>, state: &AppState) {
    use crate::backend::rpc_types::{CommandBrowserSigninOfferAnswerData, COMMAND_BROWSER_SIGNIN_OFFER_ANSWER};
    let st = state.clone();
    engine.register_typed(COMMAND_BROWSER_SIGNIN_OFFER_ANSWER, move |req: CommandBrowserSigninOfferAnswerData, ctx| {
        let state = st.clone();
        async move {
            ctx.operator_only("only the person answers an offer to save their sign-in")?;
            let answer = Answer::parse(&req.answer).ok_or_else(|| format!("answer is save, never or not_now, not {:?}", req.answer))?;
            let st = state.clone();
            tokio::task::spawn_blocking(move || self::answer(&st, &req.block_id, &req.id, answer)).await.map_err(|e| e.to_string())?
        }
    });
}

#[cfg(test)]
#[path = "browser_signin_offer_tests.rs"]
mod tests;
