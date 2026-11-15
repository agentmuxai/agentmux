// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The list of saved sign-ins under a login field
//! (SPEC_VAULT_AUTOFILL_IN_BROWSER_PANE_2026_11_14.md §P2.3, the pane-drawn route R4).
//!
//! The page's listener tells the host what the person did in a login field (a click, Up or Down,
//! a key the list takes, a press elsewhere) and the host hands it here (`credential.FieldOpen`,
//! `FieldKey`, `FieldClose`). srv owns the list: the rows the tab's profile holds for exactly that
//! frame's origin, ordered most recently used first and then by user name; the filter (what a user
//! name field holds, matched as a case-insensitive prefix of the saved user name); and which row is
//! selected. It writes what the pane draws into the block meta [`LIST_META_KEY`]: entry ids and
//! **masked** user names, never a password and never a full user name, so the pane's own text (which
//! an agent can read) says no more than the key button's does. Each answer gives the host two
//! booleans, whether a list is showing and whether a row is selected, which it sets in the listener's
//! world so the listener swallows a key only when it belongs to the list.
//!
//! The person picking a row is the approval, as for the key button: a click in the pane's own UI
//! (`browser.signin_fill`, marked operator-only) or Enter or Tab on the selected row, which the host
//! passes on only while no agent's input is reaching the pane. A pick types the sign-in through
//! [`super::browser_signin_fill::fill`], or makes a strong password through [`super::browser_signup::make`].
//!
//! Only `https`, or `http` on this computer, and never in an Incognito tab (it keeps no sign-ins).

use std::collections::HashMap;
use std::sync::LazyLock;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::backend::vault::signins;

use super::AppState;

/// The block meta key the pane reads the open list from.
pub(crate) const LIST_META_KEY: &str = "browser:signin-list";

/// The kind of login field the person is in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Username,
    Password,
    NewPassword,
}

impl Role {
    fn parse(s: &str) -> Option<Role> {
        match s {
            "username" => Some(Role::Username),
            "password" => Some(Role::Password),
            "newpassword" => Some(Role::NewPassword),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Role::Username => "username",
            Role::Password => "password",
            Role::NewPassword => "newpassword",
        }
    }
}

/// One saved sign-in in the list: `user` is the full user name (for the filter, never published).
#[derive(Clone, Debug, PartialEq)]
struct Row {
    entry: String,
    user: String,
}

/// The list the person has open in a pane.
#[derive(Clone, Debug)]
struct Open {
    origin: String,
    role: Role,
    rows: Vec<Row>,
    /// A strong password is offered (a field for a new password).
    strong: bool,
    /// The selected item: a row, or the strong password when that is all there is.
    cursor: Option<usize>,
}

impl Open {
    fn items(&self) -> usize {
        if self.strong { 1 } else { self.rows.len() }
    }
}

static LISTS: LazyLock<Mutex<HashMap<String, Open>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// What the host is told after each report: whether a list is showing, and whether a row is selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct Answer {
    pub shown: bool,
    pub cursor: bool,
}

impl Answer {
    pub(crate) fn to_json(self) -> Value {
        json!({ "shown": self.shown, "cursor": self.cursor })
    }
}

fn answer_of(open: Option<&Open>) -> Answer {
    Answer { shown: open.is_some(), cursor: open.is_some_and(|o| o.cursor.is_some()) }
}

/// The rows pane `block_id`'s profile can fill into a field of `origin`, filtered by `text` when it is a
/// user name field: most recently used first, then by user name.
fn rows_for(state: &AppState, block_id: &str, origin: &str, role: Role, text: &str) -> Vec<Row> {
    if role == Role::NewPassword || !super::browser_signin_fill::fillable(origin) {
        return Vec::new();
    }
    let Some(profile) = super::browser_signin_offer::profile_of(state, block_id) else { return Vec::new() };
    let found = match signins::offered_for(&state.identity_store, &profile, &[origin.to_string()]) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "[signin-list] couldn't read the saved sign-ins");
            return Vec::new();
        }
    };
    let typed = text.trim().to_lowercase();
    let mut rows: Vec<(i64, Row)> = found
        .into_iter()
        .filter(|(_, _, user)| role != Role::Username || typed.is_empty() || user.to_lowercase().starts_with(&typed))
        .map(|(entry, _, user)| {
            let used = state.identity_store.vault_get(&entry).ok().flatten().map_or(0, |r| r.last_used_at);
            (used, Row { entry, user })
        })
        .collect();
    rows.sort_by(|(a, ra), (b, rb)| b.cmp(a).then_with(|| ra.user.to_lowercase().cmp(&rb.user.to_lowercase())));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// The meta the pane draws from, or null for no list.
fn meta_of(open: Option<&Open>) -> Value {
    let Some(o) = open else { return Value::Null };
    json!({
        "origin": o.origin,
        "role": o.role.name(),
        "rows": o.rows.iter().map(|r| json!({ "entry": r.entry, "user": signins::mask(&r.user) })).collect::<Vec<_>>(),
        "strong": o.strong,
        "cursor": o.cursor,
    })
}

fn set_meta(state: &AppState, block_id: &str, value: Value) {
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(LIST_META_KEY.to_string(), value);
    if let Err(e) = super::http_shell::broadcast_meta_update(state, block_id, &meta) {
        tracing::warn!(error = %e, "[signin-list] couldn't show the pane's list");
    }
}

/// Show `open` (or no list) in pane `block_id`, and answer the host.
fn show(state: &AppState, block_id: &str, open: Option<Open>) -> Answer {
    let answer = answer_of(open.as_ref());
    set_meta(state, block_id, meta_of(open.as_ref()));
    match open {
        Some(o) => LISTS.lock().insert(block_id.to_string(), o),
        None => LISTS.lock().remove(block_id),
    };
    answer
}

/// The person clicked, or pressed Up or Down, in a login field of `role` in pane `block_id`'s frame of
/// `origin`; `text` is what a user name field holds.
pub(crate) fn open(state: &AppState, block_id: &str, origin: &str, role: &str, text: &str) -> Answer {
    let Some(role) = Role::parse(role) else { return close(state, block_id) };
    let rows = rows_for(state, block_id, origin, role, text);
    let strong = role == Role::NewPassword && super::browser_signin_fill::fillable(origin) && super::browser_signin_offer::profile_of(state, block_id).is_some();
    if rows.is_empty() && !strong {
        return close(state, block_id);
    }
    // A list reopened for the same field keeps its selection while the rows are the same.
    let cursor = LISTS.lock().get(block_id).filter(|o| o.origin == origin && o.role == role && o.rows == rows).and_then(|o| o.cursor);
    show(state, block_id, Some(Open { origin: origin.to_string(), role, rows, strong, cursor }))
}

/// The person pressed elsewhere on the page, or the page moved on: no list.
pub(crate) fn close(state: &AppState, block_id: &str) -> Answer {
    let was_open = LISTS.lock().contains_key(block_id);
    if was_open {
        return show(state, block_id, None);
    }
    Answer::default()
}

/// Pane `block_id`'s main frame loaded a new page.
pub(crate) fn navigated(state: &AppState, block_id: &str) {
    close(state, block_id);
}

/// The host registered: lists left in panes by an srv that forgot them go.
pub(crate) fn reset(state: &AppState) {
    LISTS.lock().clear();
    super::browser_signin_offer::clear_stale(state, LIST_META_KEY, &std::collections::BTreeSet::new(), "[signin-list]");
}

/// A key the list took: Up, Down, Enter, Tab or Escape.
pub(crate) fn key(state: &AppState, block_id: &str, key: &str) -> Answer {
    let Some(mut open) = LISTS.lock().get(block_id).cloned() else { return Answer::default() };
    let n = open.items();
    match key {
        "ArrowDown" if n > 0 => open.cursor = Some(open.cursor.map_or(0, |c| (c + 1) % n)),
        "ArrowUp" if n > 0 => open.cursor = Some(open.cursor.map_or(n - 1, |c| (c + n - 1) % n)),
        "Escape" => return close(state, block_id),
        "Enter" | "Tab" => {
            let Some(at) = open.cursor.filter(|c| *c < n) else { return answer_of(Some(&open)) };
            pick(state, block_id, &open, at);
            return close(state, block_id);
        }
        _ => {}
    }
    show(state, block_id, Some(open))
}

/// The person chose item `at` with the keyboard: do what a click on it does.
fn pick(state: &AppState, block_id: &str, open: &Open, at: usize) {
    let (state, block_id) = (state.clone(), block_id.to_string());
    if open.strong {
        tokio::spawn(async move {
            if let Err(e) = super::browser_signup::make(&state, &block_id).await {
                tracing::info!(error = %e, "[signin-list] the strong password wasn't made");
            }
        });
        return;
    }
    let Some(row) = open.rows.get(at) else { return };
    let entry = row.entry.clone();
    tokio::spawn(async move {
        if let Err(e) = super::browser_signin_fill::fill(&state, &block_id, &entry).await {
            tracing::info!(error = %e, "[signin-list] the sign-in wasn't typed");
        }
    });
}

#[cfg(test)]
#[path = "browser_signin_list_tests.rs"]
mod tests;
