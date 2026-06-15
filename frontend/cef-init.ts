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
 * Poll the window globals until the host has injected the IPC port + token, or
 * the bound elapses.
 *
 * On first load the creds arrive via URL query params and are already set when
 * this runs, so it returns immediately. On a *reload* the URL no longer carries
 * them (we strip them on first load, and a reload starts a fresh JS context
 * with empty globals); the host re-injects them into the globals via its
 * `on_load_end` hook (agentmux-cef client/mod.rs), which can land a beat after
 * setupCefApi() begins. Waiting here — once, upfront — keeps initCefApi()'s
 * first IPC call (`get_backend_endpoints`) from racing ahead of the creds,
 * failing, and falling into the 30s backend-ready wait that never resolves
 * (which left window.api unset → initApp's 5s guard fired → recovery card
 * reloaded → loop). Bounded well under that 5s guard.
 */
async function waitForIpcCreds(timeoutMs: number): Promise<void> {
    const start = performance.now();
    while (
        (window.__AGENTMUX_IPC_PORT__ == null || window.__AGENTMUX_IPC_TOKEN__ == null) &&
        performance.now() - start < timeoutMs
    ) {
        await new Promise((r) => setTimeout(r, 50));
    }
}

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

    // On a reload the creds come from the host's on_load_end re-injection, not
    // the (now-stripped) URL, and can land just after this point. Wait for them
    // here so initCefApi()'s first IPC call doesn't race ahead and wedge startup
    // (see waitForIpcCreds). No-op on first load (creds already set from URL).
    await waitForIpcCreds(2500);

    // Pre-fetch all cached values from Rust host via IPC
    await initCefApi();

    // Build the API shim and install it on window
    const api = buildCefApi();
    window.api = api;

    console.log("[cef-init] window.api installed");
}
