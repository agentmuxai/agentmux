// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tauri initialization module.
// This must run BEFORE any code that accesses window.api (getApi()).
//
// In Tauri, we populate window.api ourselves using invoke/listen.

import { buildTauriApi, initTauriApi, isTauri } from "@/util/tauri-api";

/**
 * Initialize the Tauri API shim if running inside Tauri.
 * Sets window.api to a Tauri-backed implementation of AppApi.
 *
 * This MUST be awaited before importing wave.ts or any module
 * that calls getApi() at the top level.
 */
export async function setupTauriApi(): Promise<void> {
    if (!isTauri()) {
        return; // Not running in Tauri
    }

    // Pre-fetch all cached values from Rust backend
    await initTauriApi();

    // Build the API shim and install it on window
    const api = buildTauriApi();
    (window as any).api = api;

    console.log("[tauri-init] window.api installed");
}
