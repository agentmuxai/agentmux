// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Platform IPC abstraction layer.
//
// Provides a unified interface for frontend-to-host communication.
// CEF host: HTTP POST to localhost IPC server for commands (JS→Rust),
//           CustomEvent dispatch for events (Rust→JS).

import { recordIpcRoundtrip } from "@/perf";

/**
 * Detect the current host environment.
 */
export type HostType = "cef" | "browser";

const delay = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

export function detectHost(): HostType {
    if (typeof window.__AGENTMUX_IPC_PORT__ !== "undefined") {
        return "cef";
    }
    return "browser";
}

/**
 * Invoke a host command and return the result.
 *
 * In CEF: sends HTTP POST to the local IPC server.
 * In browser: throws an error (no host available).
 */
export async function invokeCommand<T = any>(cmd: string, args?: Record<string, any>): Promise<T> {
    const host = detectHost();
    // Component C1 of perf phase 0 — record the IPC roundtrip (start
    // → host response). 16 ms = one frame at 60 Hz; the recorder
    // logs anything over that. Cost is one perf.now() pair plus a
    // map insert, well under perf budget at our call rates.
    //
    // No `typeof performance` guard: every CEF browser AgentMux
    // bundles ships the Performance API, and the prior half-guarded
    // form (guarded `t0` but unguarded `.now()` calls below) made
    // the guard a footgun rather than a safety net. If we ever ship
    // a runtime without it, fail visibly here, not silently.
    const t0 = performance.now();

    switch (host) {
        case "cef": {
            // The host re-injects fresh, self-consistent ipc_port + ipc_token into
            // the window globals on EVERY page load (on_load_end, agentmux-cef
            // client/mod.rs). On a reload there's a brief window where the first
            // IPC call can race ahead of that injection — stale/missing creds yield
            // a 401 (or no port) — which previously failed bridge init outright and
            // wedged the window in the recovery loop. Re-read the globals each
            // attempt and retry a few times so the freshly-injected creds self-heal
            // the race. Real errors (non-401 HTTP, IPC-level failures) still throw
            // immediately. See SPEC_BRIDGE_INIT_RECOVERY / token-retry follow-up.
            const MAX_ATTEMPTS = 6;
            const RETRY_DELAY_MS = 250;
            let lastErr: Error | null = null;
            for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
                // Re-read every attempt — on_load_end keeps these fresh.
                const port = window.__AGENTMUX_IPC_PORT__;
                const token = window.__AGENTMUX_IPC_TOKEN__;
                if (!port || !token) {
                    lastErr = new Error("IPC credentials not yet injected by CEF host");
                    if (attempt < MAX_ATTEMPTS) await delay(RETRY_DELAY_MS);
                    continue;
                }
                let resp: Response;
                try {
                    resp = await fetch(`http://127.0.0.1:${port}/ipc`, {
                        method: "POST",
                        headers: {
                            "Content-Type": "application/json",
                            Authorization: `Bearer ${token}`,
                        },
                        body: JSON.stringify({ cmd, args: args ?? {} }),
                    });
                } catch (e) {
                    // Network/transport error: the POST may have already reached the
                    // host and been applied before the connection dropped, so a
                    // blind retry could double-apply a mutating command (or a
                    // fire-and-forget input dispatch). Propagate instead of retrying
                    // — matching the pre-retry behavior. The retries that ARE safe
                    // (no request was sent, or the host explicitly rejected it) are
                    // the missing-creds and 401 cases above; the cred-injection race
                    // is otherwise handled upfront by waitForIpcCreds (cef-init.ts).
                    recordIpcRoundtrip(cmd, performance.now() - t0);
                    throw e instanceof Error ? e : new Error(String(e));
                }
                if (resp.status === 401) {
                    // Stale token — the host re-injects a fresh one on load. Wait,
                    // re-read the globals, and retry rather than failing the bridge.
                    lastErr = new Error("IPC unauthorized (stale token)");
                    if (attempt < MAX_ATTEMPTS) await delay(RETRY_DELAY_MS);
                    continue;
                }
                if (!resp.ok) {
                    recordIpcRoundtrip(cmd, performance.now() - t0);
                    throw new Error(`IPC HTTP error: ${resp.status} ${resp.statusText}`);
                }
                const parsed = await resp.json();
                recordIpcRoundtrip(cmd, performance.now() - t0);
                if (parsed.success) {
                    return parsed.data as T;
                }
                throw new Error(parsed.error ?? "IPC error");
            }
            recordIpcRoundtrip(cmd, performance.now() - t0);
            throw lastErr ?? new Error(`IPC '${cmd}' failed after ${MAX_ATTEMPTS} attempts`);
        }

        case "browser":
        default:
            throw new Error(
                `No host available for command '${cmd}'. ` +
                    "Running in a plain browser is not supported."
            );
    }
}

/**
 * Listen for events from the host.
 *
 * In CEF: listens for CustomEvents dispatched by the Rust host.
 * Returns an unsubscribe function.
 */
export async function listenEvent<T = any>(
    event: string,
    callback: (payload: T) => void
): Promise<() => void> {
    const host = detectHost();

    switch (host) {
        case "cef": {
            // CEF host dispatches events as:
            //   window.dispatchEvent(new CustomEvent('agentmux-event', {
            //     detail: { event: 'event-name', payload: ... }
            //   }))
            const handler = (e: Event) => {
                const detail = (e as CustomEvent).detail;
                if (detail && detail.event === event) {
                    callback(detail.payload as T);
                }
            };
            window.addEventListener("agentmux-event", handler);
            return () => window.removeEventListener("agentmux-event", handler);
        }

        case "browser":
        default:
            console.warn(`No host available for event '${event}'`);
            return () => {};
    }
}
