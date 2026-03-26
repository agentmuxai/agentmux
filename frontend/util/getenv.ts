// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

function getWindow(): Window {
    return globalThis.window;
}

function getProcess(): NodeJS.Process {
    return globalThis.process;
}

function getApi(): AppApi {
    return (window as any).api;
}

/**
 * Gets an environment variable from the host process, either directly or via IPC if called from the browser.
 * @param paramName The name of the environment variable to attempt to retrieve.
 * @returns The value of the environment variable or null if not present.
 */
export function getEnv(paramName: string): string {
    const win = getWindow();

    // In Tauri, check window globals first (set by initTauriApi)
    if (win != null) {
        const windowGlobalName = `__${paramName}__`;
        if ((win as any)[windowGlobalName] !== undefined) {
            return (win as any)[windowGlobalName];
        }
        const api = getApi();
        if (api == null) return null;
        return api.getEnv(paramName);
    }

    const proc = getProcess();
    if (proc != null) {
        return proc.env[paramName];
    }
    return null;
}
