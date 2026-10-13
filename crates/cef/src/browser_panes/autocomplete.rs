// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Address bar autocomplete for a browser pane: Chrome's own omnibox engine,
//! through the AgentMux CEF patch `CefRequestContext::GetAutocompleteSuggestions`
//! (agentmuxai/cef#12). History and typed URLs with inline completion,
//! "search for what you typed", and the default search engine's suggestions,
//! all from the pane's own profile (Personal, a named profile, or Incognito).
//!
//! A libcef without the patch allocates the stock `_cef_request_context_t`
//! (272 bytes, without the appended function pointer), so the call is skipped
//! and the answer says `supported: false`: the frontend keeps its plain address
//! bar. Same runtime-size guard as `begin_window_drag` (`ui_tasks/drag.rs`).
//!
//! Remote search suggestions (the default engine's suggest service, which
//! sees each edit before it is submitted) only when the user turned on the
//! `browser:searchsuggestions` setting. Chrome's own profile pref
//! `search.suggest_enabled` switches them, set before each query; with it off
//! only local matches come back. If the pref can't be turned off, no query
//! runs at all.
//!
//! Spike and design: docs/analysis/ANALYSIS_BROWSER_ADDRESS_BAR_AUTOCOMPLETE_SPIKE_2026_10_12.md.

use std::sync::Arc;
use std::time::Duration;

use cef::rc::{ConvertReturnValue, RcImpl};
use cef::sys::{_cef_autocomplete_callback_t, _cef_list_value_t, _cef_request_context_t, _cef_string_utf16_t};
use cef::*;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::oneshot;

use crate::state::AppState;

use super::BrowserPaneManager;

/// Chrome's pref for the default search engine's live suggestions.
const SEARCH_SUGGEST_PREF: &str = "search.suggest_enabled";

/// How long a query may take before the frontend gets the newest partial set.
/// The local providers answer within milliseconds; this only bounds a slow
/// network suggestion (libcef itself gives up after 10 s).
const RESULT_WAIT: Duration = Duration::from_millis(2500);

/// One suggestion, as the address bar shows it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AutocompleteMatch {
    pub contents: String,
    pub description: String,
    /// Where choosing it goes: a page, or a search results URL.
    pub url: String,
    pub fill_into_edit: String,
    /// Text to append after what was typed when this is the default match.
    pub inline_autocompletion: String,
    /// Chrome's match type, e.g. `history-url`, `search-suggest`.
    #[serde(rename = "type")]
    pub kind: String,
    pub provider: String,
    pub relevance: i32,
    pub allowed_to_be_default: bool,
}

/// `None`: this libcef has no autocomplete API.
type Outcome = Option<Vec<AutocompleteMatch>>;

/// Shared between the waiting IPC call and the CEF callback.
#[derive(Default)]
struct Pending {
    /// Answers the IPC call: on the final result, or "unsupported".
    tx: Mutex<Option<oneshot::Sender<Outcome>>>,
    /// The newest result set, for an answer that times out first.
    latest: Mutex<Vec<AutocompleteMatch>>,
}

impl Pending {
    fn answer(&self, outcome: Outcome) {
        if let Some(tx) = self.tx.lock().take() {
            let _ = tx.send(outcome);
        }
    }
}

impl BrowserPaneManager {
    /// Chrome's address bar suggestions for `text` in `block_id`'s profile.
    /// `{"supported": false}` when the pane's libcef lacks the API.
    /// `search_suggestions`: also ask the default search engine (the setting).
    pub async fn autocomplete(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
        text: &str,
        search_suggestions: bool,
    ) -> serde_json::Value {
        let (tx, rx) = oneshot::channel();
        let pending = Arc::new(Pending { tx: Mutex::new(Some(tx)), ..Default::default() });
        let mut task = StartAutocompleteTask::new(
            state.clone(),
            block_id.to_string(),
            text.to_string(),
            search_suggestions,
            pending.clone(),
        );
        if post_task(ThreadId::UI, Some(&mut task)) == 0 {
            // No UI thread (shutting down).
            return serde_json::json!({ "supported": true, "matches": [] });
        }
        let outcome = match tokio::time::timeout(RESULT_WAIT, rx).await {
            Ok(Ok(outcome)) => outcome,
            // Timed out, or the callback went away without a final call.
            _ => Some(pending.latest.lock().clone()),
        };
        match outcome {
            None => serde_json::json!({ "supported": false, "matches": [] }),
            Some(matches) => serde_json::json!({ "supported": true, "matches": matches }),
        }
    }
}

wrap_task! {
    pub struct StartAutocompleteTask {
        state: Arc<AppState>,
        block_id: String,
        text: String,
        search_suggestions: bool,
        pending: Arc<Pending>,
    }

    impl Task {
        fn execute(&self) {
            let Some(browser) = self.state.browser_panes.live_browser(&self.state, &self.block_id) else {
                self.pending.answer(Some(Vec::new()));
                return;
            };
            if !start(&browser, &self.text, self.search_suggestions, self.pending.clone()) {
                self.pending.answer(None);
            }
        }
    }
}

/// Start the query on the UI thread. False when this libcef has no API for it.
fn start(browser: &Browser, text: &str, search_suggestions: bool, pending: Arc<Pending>) -> bool {
    let Some(context) = browser.host().and_then(|host| host.request_context()) else {
        pending.answer(Some(Vec::new()));
        return true;
    };
    let raw: *mut _cef_request_context_t = ImplRequestContext::get_raw(&context);
    unsafe {
        // The struct libcef allocated: a stock build's ends before our field.
        let runtime_size = (*raw).base.base.size;
        if runtime_size < std::mem::size_of::<_cef_request_context_t>() {
            tracing::info!(runtime_size, "[autocomplete] libcef has no GetAutocompleteSuggestions (stock build)");
            return false;
        }
        let Some(get_autocomplete_suggestions) = (*raw).get_autocomplete_suggestions else {
            return false;
        };
        if !set_search_suggestions(&context, search_suggestions) && !search_suggestions {
            // Fail closed: never let typed text reach the engine against the setting.
            pending.answer(Some(Vec::new()));
            return true;
        }
        let callback = RcImpl::new(
            _cef_autocomplete_callback_t {
                // RcImpl::new fills in the base (size and reference counting).
                base: std::mem::zeroed(),
                on_autocomplete_result: Some(on_autocomplete_result),
            },
            pending,
        );
        let text = CefString::from(text);
        let text_raw: *const _cef_string_utf16_t = (&text).into();
        // Hands libcef the callback's one reference; it releases it when done.
        get_autocomplete_suggestions(raw, text_raw, callback.cast());
    }
    true
}

/// Switch the profile's live search suggestions to `on`, unless already so.
/// False when the pref couldn't be read or written.
fn set_search_suggestions(context: &RequestContext, on: bool) -> bool {
    let name = CefString::from(SEARCH_SUGGEST_PREF);
    if let Some(current) = ImplPreferenceManager::preference(context, Some(&name)) {
        if (current.bool() != 0) == on {
            return true;
        }
    }
    let Some(mut value) = value_create() else {
        return false;
    };
    value.set_bool(on as std::os::raw::c_int);
    let mut error = CefString::default();
    if ImplPreferenceManager::set_preference(context, Some(&name), Some(&mut value), Some(&mut error)) == 0 {
        tracing::warn!(on, error = %error, "[autocomplete] couldn't set {SEARCH_SUGGEST_PREF}");
        return false;
    }
    true
}

/// `_cef_autocomplete_callback_t::on_autocomplete_result`, on the UI thread.
unsafe extern "C" fn on_autocomplete_result(
    self_: *mut _cef_autocomplete_callback_t,
    matches: *mut _cef_list_value_t,
    done: std::os::raw::c_int,
) {
    let this: &RcImpl<_cef_autocomplete_callback_t, Arc<Pending>> = RcImpl::get(self_);
    // libcef passes the list with a reference for us; ListValue releases it.
    let parsed = if matches.is_null() {
        Vec::new()
    } else {
        parse_matches(&matches.wrap_result())
    };
    *this.interface.latest.lock() = parsed.clone();
    if done != 0 {
        this.interface.answer(Some(parsed));
    }
}

fn parse_matches(list: &ListValue) -> Vec<AutocompleteMatch> {
    (0..list.size())
        .filter_map(|i| list.dictionary(i))
        .map(|m| {
            let string = |key: &str| CefString::from(&m.string(Some(&CefString::from(key)))).to_string();
            AutocompleteMatch {
                contents: string("contents"),
                description: string("description"),
                url: string("destination_url"),
                fill_into_edit: string("fill_into_edit"),
                inline_autocompletion: string("inline_autocompletion"),
                kind: string("type"),
                provider: string("provider"),
                relevance: m.int(Some(&CefString::from("relevance"))),
                allowed_to_be_default: m.bool(Some(&CefString::from("allowed_to_be_default_match"))) != 0,
            }
        })
        .collect()
}
