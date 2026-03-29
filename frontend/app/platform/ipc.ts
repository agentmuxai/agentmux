// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Platform IPC abstraction layer.
//
// Provides a unified interface for frontend-to-host communication that works
// with both the Tauri host (invoke/listen) and the CEF host (cefQuery).
// The frontend calls these functions without knowing which host it's running in.
//
// Phase 1: Tauri path is fully functional, CEF path is a placeholder.
// Phase 2: CEF path will use window.cefQuery() for JS→Rust and
//          window.dispatchEvent() for Rust→JS.

/**
 * Detect the current host environment.
 */
export type HostType = "tauri" | "cef" | "browser";

export function detectHost(): HostType {
    if (typeof (window as any).__TAURI_INTERNALS__ !== "undefined") {
        return "tauri";
    }
    if (typeof (window as any).cefQuery !== "undefined") {
        return "cef";
    }
    return "browser";
}

/**
 * Invoke a host command and return the result.
 *
 * In Tauri: delegates to @tauri-apps/api/core invoke().
 * In CEF: sends a cefQuery message to the Rust handler (Phase 2).
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
            return new Promise<T>((resolve, reject) => {
                const cefQuery = (window as any).cefQuery;
                if (!cefQuery) {
                    reject(new Error("cefQuery not available"));
                    return;
                }
                cefQuery({
                    request: JSON.stringify({ cmd, args: args ?? {} }),
                    onSuccess: (response: string) => {
                        try {
                            const parsed = JSON.parse(response);
                            if (parsed.success) {
                                resolve(parsed.data as T);
                            } else {
                                reject(new Error(parsed.data?.error ?? "Unknown error"));
                            }
                        } catch {
                            // If response isn't JSON, return it as-is
                            resolve(response as unknown as T);
                        }
                    },
                    onFailure: (_code: number, message: string) => {
                        reject(new Error(message));
                    },
                });
            });
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
