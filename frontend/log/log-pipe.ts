// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Console log pipe: monkey-patches console.log/warn/error/debug/info
// to forward all messages to the Rust host via the fe_log_structured
// IPC command. Original console behavior is preserved.
//
// Usage: call initLogPipe() once at startup, before any other code.

import { invokeCommand } from "@/app/platform/ipc";

const LEVELS = ["log", "warn", "error", "debug", "info"] as const;
type LogLevel = (typeof LEVELS)[number];

let initialized = false;

export function initLogPipe() {
    if (initialized) return;
    initialized = true;

    for (const level of LEVELS) {
        const original = console[level].bind(console);
        console[level] = (...args: any[]) => {
            // Always call the original so DevTools works normally
            original(...args);

            try {
                const msg = args
                    .map((a) => {
                        if (typeof a === "string") return a;
                        try {
                            return JSON.stringify(a);
                        } catch {
                            return String(a);
                        }
                    })
                    .join(" ");

                // Fire-and-forget — never let logging break the app
                invokeCommand("fe_log_structured", {
                    level: level === "log" ? "info" : level,
                    module: "console",
                    message: msg,
                    data: null,
                }).catch(() => {});
            } catch {
                // swallow
            }
        };
    }
}
