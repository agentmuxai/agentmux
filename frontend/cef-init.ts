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
 * The host injects the port and token into the globals as the page starts
 * loading (`on_load_start`, again at `on_load_end`; agentmux-cef
 * client/ipc_credentials.rs); the address carries no token. Injection normally
 * lands before this runs, but it can land a beat after setupCefApi() begins.
 * Waiting here — once, upfront — keeps initCefApi()'s
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

    // The port rides in the address as a hint; the token never does (the host
    // injects it). A token an older host still put in the address is taken and
    // stripped like the port.
    const params = new URLSearchParams(window.location.search);
    const port = params.get("ipc_port");
    const token = params.get("ipc_token");
    if (port && window.__AGENTMUX_IPC_PORT__ == null) {
        window.__AGENTMUX_IPC_PORT__ = parseInt(port, 10);
    }
    if (token) {
        window.__AGENTMUX_IPC_TOKEN__ = token;
    }

    // Keep the address clean: strip the IPC params once read. Other params (e.g.
    // windowLabel) are preserved.
    if (port || token) {
        const url = new URL(window.location.href);
        url.searchParams.delete("ipc_token");
        url.searchParams.delete("ipc_port");
        window.history.replaceState(window.history.state, "", url.toString());
    }

    // The creds come from the host's injection and can land just after this
    // point. Wait for them here so initCefApi()'s first IPC call doesn't race
    // ahead and wedge startup (see waitForIpcCreds); bounded under the 5 s
    // window.api guard.
    await waitForIpcCreds(4000);

    // Pre-fetch all cached values from Rust host via IPC
    await initCefApi();

    // Build the API shim and install it on window
    const api = buildCefApi();
    window.api = api;

    console.log("[cef-init] window.api installed");
}
