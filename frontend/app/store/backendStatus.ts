// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Backend process status signals and event listener setup.
// Kept separate from global.ts so it can be imported and tested in isolation
// without pulling in layout, wshrpcutil, or other heavy dependencies.

import { createSignal } from "solid-js";

export type BackendStatusState = "connecting" | "running" | "crashed";

export interface BackendDeathInfo {
    code: number | null;
    signal: number | null;
    pid: number;
    uptime_secs: number | null;
    died_at: string; // ISO timestamp set by frontend at receipt
}

export const [backendStatusAtom, setBackendStatusAtom] = createSignal<BackendStatusState>("connecting");
export const [backendDeathInfoAtom, setBackendDeathInfoAtom] = createSignal<BackendDeathInfo | null>(null);

/// Set to true by handleRestart before issuing the restart command.
/// Prevents the intermediate backend-terminated event from overriding "connecting".
/// Cleared automatically when backend-ready fires, or by the caller on error.
const [restartInProgress, setRestartInProgress] = createSignal(false);
export { setRestartInProgress };

/// Minimal slice of AppApi that this module needs.
/// Using a narrow interface keeps the dependency injectable and mockable in tests.
interface BackendStatusApi {
    listen: (event: string, callback: (event: any) => void) => Promise<() => void>;
    getBackendInfo: () => Promise<{ pid?: number; started_at?: string; web_endpoint?: string; version: string }>;
}

/// Wire up the Tauri event listeners that drive backendStatusAtom.
///
/// Call once during app init (from initGlobalSignals).
/// The api and reconnectWS parameters are injected so tests can pass mocks
/// without any vi.mock() setup.
export function initBackendStatusListeners(
    api: BackendStatusApi,
    reconnectWS: (newEndpoint: string) => void,
): void {
    api.listen("backend-terminated", (event) => {
        // During a user-initiated restart the old sidecar is killed deliberately.
        // Suppress the crashed transition so the UI stays on "connecting".
        if (restartInProgress()) return;
        const p = (event as { payload?: Partial<BackendDeathInfo> }).payload ?? {};
        setBackendDeathInfoAtom({
            code: p.code ?? null,
            signal: p.signal ?? null,
            pid: p.pid ?? 0,
            uptime_secs: p.uptime_secs ?? null,
            died_at: new Date().toISOString(),
        });
        setBackendStatusAtom("crashed");
    });

    api.listen("backend-ready", (event) => {
        setRestartInProgress(false); // clear flag — new backend is up
        const payload = (event as any)?.payload as { ws?: string; web?: string } | null;
        if (payload?.ws) {
            // Update window globals so getWSServerEndpoint() returns the new address
            window.__WAVE_SERVER_WS_ENDPOINT__ = payload.ws;
            window.__WAVE_SERVER_WEB_ENDPOINT__ = payload.web ?? "";
            // Reconnect the WS client to the new endpoint (port may have changed)
            reconnectWS(`ws://${payload.ws}`);
        }
        setBackendStatusAtom("running");
    });

    // The backend-ready event may have already fired before this listener was
    // registered (when initTauriApi resolved via invoke rather than waiting for
    // the event). Catch up by checking whether the backend is already up.
    api.getBackendInfo().then(() => {
        if (backendStatusAtom() === "connecting") {
            setBackendStatusAtom("running");
        }
    }).catch(() => {});
}
