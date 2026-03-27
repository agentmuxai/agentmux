// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Utility to abstract the fetch function.
// Uses standard fetch API. CORS handled via tauri.conf.json security settings.

export function fetch(input: string | Request | URL, init?: RequestInit): Promise<Response> {
    // Always use globalThis.fetch (standard Web API)
    // Tauri provides fetch API in the webview
    return globalThis.fetch(input, init);
}
