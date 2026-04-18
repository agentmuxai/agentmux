// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Detect whether we're running inside the AgentMux desktop app.
 * The host app owns the backend sidecar and sets __AGENTMUX_IPC_PORT__
 * as a URL query parameter when loading the frontend.
 */
export function isHostApp(): boolean {
    return typeof window.__AGENTMUX_IPC_PORT__ !== "undefined";
}
