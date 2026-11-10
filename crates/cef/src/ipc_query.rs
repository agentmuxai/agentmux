// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A process-message lane for the one host command the page sends every frame.
//!
//! Host commands go over HTTP (`ipc.rs`): the page `fetch`es `/ipc` and axum
//! answers. That costs 7 to 9 ms a round trip and one of the page's six
//! connections to the host. The browser panes' rect batch
//! (`browser_panes_set_rects`: one per window per frame while a pane border or
//! the window is dragged) paid it on every frame, so a pane's body trailed the
//! chrome drawn around it (ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md
//! §5.1, "Transport"; task #46447).
//!
//! This module runs that command over cef-rs's message router instead. The
//! renderer puts `window.agentmuxQuery(...)` on an app window's main frame; a
//! call is one process message to the browser process, [`PaneRectsHandler`]
//! takes it on the UI thread, and the answer is one message back. The page
//! uses the lane when its window has it and HTTP otherwise
//! (`frontend/app/platform/ipc-query.ts`), so a window without the function,
//! an older host, or a query the host refuses all still move the panes, as
//! before.
//!
//! The lane carries `browser_panes_set_rects` only. A query must carry the IPC
//! token `/ipc` requires (the page has it in `window.__AGENTMUX_IPC_TOKEN__`; a
//! pane's page doesn't), and must not come from a browser pane. The function
//! is only put on the main frame of a browser created with [`app_extra_info`],
//! which the app's windows pass at creation, so a pane's page never sees it.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex as StdMutex, OnceLock};

use cef::wrapper::message_router::{
    BrowserSideCallback, BrowserSideHandler, BrowserSideRouter, MessageRouterBrowserSide,
    MessageRouterBrowserSideHandlerCallbacks, MessageRouterConfig, MessageRouterRendererSide,
    MessageRouterRendererSideHandlerCallbacks, RendererSideRouter,
};
use cef::*;
use parking_lot::Mutex;

use crate::browser_panes::bounds::{self, Waiter};
use crate::state::AppState;

/// The JavaScript function the renderer adds to an app window.
pub const QUERY_FUNCTION: &str = "agentmuxQuery";
const CANCEL_FUNCTION: &str = "agentmuxQueryCancel";
/// The `extra_info` key an app window's browser is created with.
const APP_KEY: &str = "agentmux_app";
/// The one command this lane carries.
const SET_RECTS: &str = "browser_panes_set_rects";

/// Why a query the handler took was refused (`onFailure`'s code). The router's
/// own -1 means no handler took the query at all.
const BAD_REQUEST: i32 = 400;
const UNAUTHORIZED: i32 = 401;
const NOT_AN_APP_WINDOW: i32 = 403;

/// The same values on both sides, or the renderer's messages go unanswered.
fn config() -> MessageRouterConfig {
    MessageRouterConfig {
        js_query_function: QUERY_FUNCTION.to_string(),
        js_cancel_function: CANCEL_FUNCTION.to_string(),
        ..Default::default()
    }
}

/// The `extra_info` an app window's browser is created with: the renderer
/// reads it in `on_browser_created` and puts the lane on that browser only.
pub fn app_extra_info() -> Option<DictionaryValue> {
    let info = dictionary_value_create()?;
    info.set_bool(Some(&CefString::from(APP_KEY)), 1);
    Some(info)
}

fn is_app(extra_info: Option<&DictionaryValue>) -> bool {
    extra_info.map(|d| d.bool(Some(&CefString::from(APP_KEY))) != 0).unwrap_or(false)
}

/// What a query asks: `{ cmd, token, args }`.
#[derive(serde::Deserialize)]
struct Query {
    cmd: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    args: serde_json::Value,
}

/// Screen a query's text. `None`: not ours (the router cancels it with -1 and
/// the page uses HTTP). Otherwise the batch's args, or the code to refuse with.
fn screen(request: &str, ipc_token: &str, forward_token: &str) -> Option<Result<serde_json::Value, i32>> {
    let q: Query = serde_json::from_str(request).ok()?;
    if q.cmd != SET_RECTS {
        return None;
    }
    let access = crate::ipc_forward::access_for(Some(&q.token), ipc_token, forward_token);
    Some(if crate::ipc_forward::allows(access, &q.cmd) { Ok(q.args) } else { Err(UNAUTHORIZED) })
}

fn fail(callback: &Arc<StdMutex<dyn BrowserSideCallback>>, code: i32, message: &str) {
    if let Ok(cb) = callback.lock() {
        cb.failure(code, message);
    }
}

/// The browser side: takes `browser_panes_set_rects` queries on the UI thread.
struct PaneRectsHandler {
    state: Arc<AppState>,
}

impl BrowserSideHandler for PaneRectsHandler {
    fn on_query_str(
        &self,
        browser: Option<Browser>,
        _frame: Option<Frame>,
        _query_id: i64,
        request: &str,
        _persistent: bool,
        callback: Arc<StdMutex<dyn BrowserSideCallback>>,
    ) -> bool {
        let args = match screen(request, &self.state.ipc_token, crate::ipc_forward::forward_token()) {
            None => return false,
            Some(Err(code)) => {
                fail(&callback, code, "the query's token is not this host's");
                return true;
            }
            Some(Ok(args)) => args,
        };
        // A pane's page never gets the function, but the token is what admits a
        // query; a pane is refused whatever it presents.
        let mut browser = browser;
        let from_pane = browser
            .as_mut()
            .and_then(|b| crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b))
            .is_some();
        if from_pane {
            fail(&callback, NOT_AN_APP_WINDOW, "a browser pane can't move panes");
            return true;
        }
        match bounds::parse_set_rects_args(&args) {
            Ok((window_label, rects)) => {
                let base = bounds::LayoutBase::parse(&args);
                let order = bounds::RectsOrder::parse(&args);
                bounds::queue(&self.state, &window_label, rects, base, order, Waiter::Query(callback));
            }
            Err(e) => fail(&callback, BAD_REQUEST, &e),
        }
        true
    }
}

static ROUTER: OnceLock<Arc<BrowserSideRouter>> = OnceLock::new();

/// The browser-side router, made on first use. UI thread (`add_handler`).
fn router(state: &Arc<AppState>) -> &'static Arc<BrowserSideRouter> {
    ROUTER.get_or_init(|| {
        let router = BrowserSideRouter::new(config());
        router.add_handler(Arc::new(PaneRectsHandler { state: state.clone() }), true);
        router
    })
}

/// From `Client::on_process_message_received` (UI thread). True when the
/// message was a query or a cancel for this lane.
pub fn on_process_message_received(
    state: &Arc<AppState>,
    browser: Option<&Browser>,
    frame: Option<&Frame>,
    source: ProcessId,
    message: Option<&ProcessMessage>,
) -> bool {
    router(state).on_process_message_received(browser.cloned(), frame.cloned(), source, message.cloned())
}

/// From `RequestHandler::on_before_browse`, for a navigation that goes ahead:
/// a main-frame navigation ends the page's pending queries.
pub fn on_before_browse(browser: Option<&Browser>, frame: Option<&Frame>) {
    if let Some(r) = ROUTER.get() {
        r.on_before_browse(browser.cloned(), frame.cloned());
    }
}

/// From `RequestHandler::on_render_process_terminated`.
pub fn on_render_process_terminated(browser: Option<&Browser>) {
    if let Some(r) = ROUTER.get() {
        r.on_render_process_terminated(browser.cloned());
    }
}

/// From `LifeSpanHandler::on_before_close`.
pub fn on_before_close(browser: Option<&Browser>) {
    if let Some(r) = ROUTER.get() {
        r.on_before_close(browser.cloned());
    }
}

/// The renderer side, called from the subprocess app's render-process handler
/// (`renderer_map::PidReporter`). Everything here runs on the renderer's main
/// thread.
pub mod renderer {
    use super::*;

    static ROUTER: OnceLock<Arc<RendererSideRouter>> = OnceLock::new();
    /// The browsers in this renderer created with [`app_extra_info`].
    static APP_BROWSERS: LazyLock<Mutex<HashSet<i32>>> = LazyLock::new(Default::default);

    fn router() -> &'static Arc<RendererSideRouter> {
        ROUTER.get_or_init(|| RendererSideRouter::new(config()))
    }

    pub fn on_browser_created(browser: Option<&Browser>, extra_info: Option<&DictionaryValue>) {
        if let (Some(b), true) = (browser, is_app(extra_info)) {
            APP_BROWSERS.lock().insert(b.identifier());
        }
    }

    pub fn on_browser_destroyed(browser: Option<&Browser>) {
        if let Some(b) = browser {
            APP_BROWSERS.lock().remove(&b.identifier());
        }
    }

    /// Puts the query function on the context: an app window's main frame only.
    pub fn on_context_created(browser: Option<&Browser>, frame: Option<&Frame>, context: Option<&V8Context>) {
        let (Some(b), Some(f)) = (browser, frame) else { return };
        if f.is_main() == 0 || !APP_BROWSERS.lock().contains(&b.identifier()) {
            return;
        }
        router().on_context_created(Some(b.clone()), Some(f.clone()), context.cloned());
    }

    pub fn on_context_released(browser: Option<&Browser>, frame: Option<&Frame>, context: Option<&V8Context>) {
        if let Some(r) = ROUTER.get() {
            r.on_context_released(browser.cloned(), frame.cloned(), context.cloned());
        }
    }

    /// The browser process's answer to a query. True when it was one.
    pub fn on_process_message_received(
        browser: Option<&Browser>,
        frame: Option<&Frame>,
        source: ProcessId,
        message: Option<&ProcessMessage>,
    ) -> bool {
        ROUTER
            .get()
            .map(|r| r.on_process_message_received(browser.cloned(), frame.cloned(), Some(source), message.cloned()))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "the-host-token";
    const FORWARD: &str = "the-forward-token";

    fn query(cmd: &str, token: &str) -> String {
        serde_json::json!({ "cmd": cmd, "token": token, "args": { "window_label": "main", "rects": [] } }).to_string()
    }

    #[test]
    fn a_set_rects_query_with_the_hosts_token_passes_with_its_args() {
        let args = screen(&query(SET_RECTS, TOKEN), TOKEN, FORWARD).unwrap().unwrap();
        assert_eq!(args["window_label"], "main");
    }

    #[test]
    fn a_wrong_or_missing_token_is_refused_as_unauthorized() {
        assert_eq!(screen(&query(SET_RECTS, "guess"), TOKEN, FORWARD), Some(Err(UNAUTHORIZED)));
        let no_token = serde_json::json!({ "cmd": SET_RECTS, "args": {} }).to_string();
        assert_eq!(screen(&no_token, TOKEN, FORWARD), Some(Err(UNAUTHORIZED)));
    }

    #[test]
    fn the_forwarding_token_does_not_open_the_lane() {
        // The launcher's token is for its own few commands (ipc_forward), not this one.
        assert_eq!(screen(&query(SET_RECTS, FORWARD), TOKEN, FORWARD), Some(Err(UNAUTHORIZED)));
    }

    #[test]
    fn any_other_command_and_any_other_text_is_not_ours() {
        assert!(screen(&query("get_platform", TOKEN), TOKEN, FORWARD).is_none());
        assert!(screen("not json", TOKEN, FORWARD).is_none());
        assert!(screen("{\"token\":\"x\"}", TOKEN, FORWARD).is_none());
    }
}
