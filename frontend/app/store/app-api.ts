// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Accessor for the host API bridge (window.api / Tauri IPC).
// Extracted here so it can be imported by modules below the store layer
// (e.g. util/logger.ts, store/wos.ts) without creating a cycle through global.ts.

export function getApi(): AppApi {
    if (!window.api) {
        console.error("[getApi] called before window.api exists. Stack:", new Error().stack);
    }
    return window.api;
}
