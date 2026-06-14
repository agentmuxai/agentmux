// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CEF initialization module.
// This must run BEFORE any code that accesses window.api (getApi()).
//
// In CEF, we populate window.api ourselves using invokeCommand/listenEvent
// via the embedded HTTP IPC server.

import { buildCefApi, initCefApi, isCef } from "@/util/cef-api";

/**
 * Initialize the CEF API shim if running inside a CEF host.
 * Sets window.api to a CEF-backed implementation of AppApi.
 *
 * This MUST be awaited before importing app-init.ts or any module
 * that calls getApi() at the top level.
 */
export async function setupCefApi(): Promise<void> {
    if (!isCef()) {
        return; // Not running in CEF
    }

    // Set IPC globals from URL query params so invokeCommand() can find them.
    const params = new URLSearchParams(window.location.search);
    const port = params.get("ipc_port");
    const token = params.get("ipc_token");
    if (port) {
        window.__AGENTMUX_IPC_PORT__ = parseInt(port, 10);
    }
    if (token) {
        window.__AGENTMUX_IPC_TOKEN__ = token;
    }

    // Security: strip the IPC port/token from the visible URL immediately after
    // capturing them into window globals. Leaving them in window.location.href
    // exposes the bearer token for the whole page lifetime (referrer leakage to
    // remote browser-pane origins, devtools, crash dumps). Subsequent reads use
    // the window globals above, not the URL. Other params (e.g. windowLabel) are
    // preserved. See reports security sweep 2026-06-12 (ipc-token-in-url).
    if (port || token) {
        const url = new URL(window.location.href);
        url.searchParams.delete("ipc_token");
        url.searchParams.delete("ipc_port");
        window.history.replaceState(window.history.state, "", url.toString());
    }

    // Pre-fetch all cached values from Rust host via IPC
    await initCefApi();

    // Build the API shim and install it on window
    const api = buildCefApi();
    window.api = api;

    console.log("[cef-init] window.api installed");
}
