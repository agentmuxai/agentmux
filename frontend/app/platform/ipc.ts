// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Platform IPC abstraction layer.
//
// Provides a unified interface for frontend-to-host communication that works
// with both the Tauri host (invoke/listen) and the CEF host (HTTP fetch).
// The frontend calls these functions without knowing which host it's running in.
//
// Tauri: uses @tauri-apps/api invoke()/listen().
// CEF:   uses HTTP POST to localhost IPC server for commands (JS→Rust),
//        and CustomEvent dispatch for events (Rust→JS).

/**
 * Detect the current host environment.
 */
export type HostType = "tauri" | "cef" | "browser";

export function detectHost(): HostType {
    if (typeof (window as any).__TAURI_INTERNALS__ !== "undefined") {
        return "tauri";
    }
    if (typeof (window as any).__AGENTMUX_IPC_PORT__ !== "undefined") {
        return "cef";
    }
    return "browser";
}

/**
 * Invoke a host command and return the result.
 *
 * In Tauri: delegates to @tauri-apps/api/core invoke().
 * In CEF: sends HTTP POST to the local IPC server.
 * In browser: throws an error (no host available).
 */
export async function invokeCommand<T = any>(cmd: string, args?: Record<string, any>): Promise<T> {
    const host = detectHost();

    switch (host) {
        case "tauri": {
            const { invoke } = await import("@tauri-apps/api/core");
            return invoke<T>(cmd, args);
        }

        case "cef": {
            const port = (window as any).__AGENTMUX_IPC_PORT__;
            if (!port) {
                throw new Error("IPC port not injected by CEF host");
            }
            const resp = await fetch(`http://127.0.0.1:${port}/ipc`, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ cmd, args: args ?? {} }),
            });
            if (!resp.ok) {
                throw new Error(`IPC HTTP error: ${resp.status} ${resp.statusText}`);
            }
            const parsed = await resp.json();
            if (parsed.success) {
                return parsed.data as T;
            }
            throw new Error(parsed.error ?? "IPC error");
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
 * In Tauri: delegates to @tauri-apps/api/event listen().
 * In CEF: listens for CustomEvents dispatched by the Rust host (Phase 2).
 * Returns an unsubscribe function.
 */
export async function listenEvent<T = any>(
    event: string,
    callback: (payload: T) => void
): Promise<() => void> {
    const host = detectHost();

    switch (host) {
        case "tauri": {
            const { listen } = await import("@tauri-apps/api/event");
            const unlisten = await listen<T>(event, (e) => callback(e.payload));
            return unlisten;
        }

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
