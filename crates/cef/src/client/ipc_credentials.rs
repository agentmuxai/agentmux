// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Handing the host's IPC credentials to the frontend's pages.
//!
//! The window's address carries no token: the host puts the port and token into
//! the page's globals itself, as the page starts loading (`on_load_start`, so
//! they are there before the app's scripts need them) and again when the load
//! ends (`on_load_end`, a safety net for a page that began before its browser
//! was registered). Only the frontend's own pages get them (see
//! `app_navigation::injects_ipc_credentials`).

use cef::*;

use super::AgentMuxHandler;

impl AgentMuxHandler {
    /// Injects `window.__AGENTMUX_IPC_PORT__` / `__AGENTMUX_IPC_TOKEN__` into the
    /// browser's main frame when it shows the frontend.
    pub(crate) fn inject_ipc_credentials(&self, browser: &Browser) {
        // Re-inject the host IPC creds on EVERY main-frame load of OUR OWN
        // frontend — crucially INCLUDING `is_browser_pane` floating-pane /
        // pool windows, which host our frontend (on our localhost origin)
        // but were starved of creds by the old `!is_browser_pane` gate.
        //
        // Why every load, not just the first: the frontend strips
        // `ipc_port`/`ipc_token` from the URL after reading them once
        // (cef-init.ts token-leak fix), so any reload (Vite HMR, WebGL
        // context-loss reload, or the bridge auto-recover itself) arrives
        // cred-less. Without re-injection, `setupCefApi` can't rebuild
        // `window.api` → the permanent "window.api still undefined after 5s"
        // blank + ~5s reload storm observed on floating-pool windows (#52).
        //
        // Gate on frame ORIGIN, not the pane flag: inject only when the
        // loading main frame is on our frontend origin. A real browser pane
        // rendering a remote site (example.com) is a different origin and
        // still never receives the bearer token — preserving exactly the
        // leak protection the `is_browser_pane` gate was meant to provide.
        let Some(frame) = browser.main_frame() else { return };
        let frame_url = CefString::from(&frame.url()).to_string();
        // Authoritative port (self.ipc_port is 0 for floating-pane/pool
        // handlers — see resolved_ipc_port). Injecting 0 would make the
        // frontend's invokeCommand reject the creds and time out on
        // get_backend_endpoints (#52).
        let ipc_port = self.resolved_ipc_port();
        // Only the frontend's own pages get the creds, in every window: an
        // app window that somehow loaded another site, or a browser pane on a
        // remote one, gets nothing (app_navigation.rs).
        // A browser pane gets none, even when it is showing the frontend's address: that is
        // decided from what the browser is registered as. A browser not registered yet falls
        // back to the origin alone, as before; a pane is registered when it is created.
        let registered_as_pane = super::pane_frontend_guard::shows_web_pages(self.registered_kind(browser).as_ref());
        let should_inject =
            super::app_navigation::injects_ipc_credentials(&frame_url, &self.frontend_origins(), registered_as_pane);
        if registered_as_pane && super::app_navigation::is_frontend_address(&frame_url, &self.frontend_origins()) {
            tracing::warn!("a browser pane is showing the app's own address: it gets no IPC credentials");
        }
        if should_inject {
            let ipc_token = &self.state.ipc_token;
            let js = format!(
                "window.__AGENTMUX_IPC_PORT__ = {}; window.__AGENTMUX_IPC_TOKEN__ = '{}';",
                ipc_port, ipc_token
            );
            let code = CefString::from(js.as_str());
            let empty = CefString::from("");
            frame.execute_java_script(Some(&code), Some(&empty), 0);
            tracing::info!(
                "Injected IPC port {} into page: {}",
                ipc_port,
                super::app_navigation::redact_ipc_token(&frame_url)
            );
        }
    }
}
