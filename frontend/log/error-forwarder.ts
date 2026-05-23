// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Uncaught error forwarder: captures `window.error` and
// `window.unhandledrejection` events and forwards them to the Rust host
// via the same `fe_log_structured` IPC channel that `log-pipe.ts` uses
// for monkey-patched console.* calls.
//
// Why this exists (retro 2026-05-23 — agent-pane cascade → replaceChild
// quick-win): DOM exceptions thrown from SolidJS's reconciler (e.g.
// `NotFoundError: The node to be removed is not a child of this node`)
// bypass the console pipe entirely — they surface only as window-level
// "error" events. Without a global handler that forwards them through
// the same IPC channel, the host log shows no trace of the throw,
// only the cascade warnings that preceded it.
//
// This forwarder is purely a logging side-effect:
//   - it does NOT call event.preventDefault() (errors still surface in
//     DevTools and SolidJS ErrorBoundary)
//   - it does NOT swallow the error
//   - it is fire-and-forget on the IPC side, so a failing send never
//     breaks the app
//
// Usage: call initErrorForwarder() once at startup, ideally right after
// initLogPipe() so any errors during the rest of bootstrap are caught.

import { invokeCommand } from "@/app/platform/ipc";

let initialized = false;

interface ForwardedErrorPayload {
    message: string;
    stack: string | null;
    name: string | null;
    source: string | null;
}

function safeStringify(value: unknown): string {
    if (value === null) return "null";
    if (value === undefined) return "undefined";
    if (typeof value === "string") return value;
    try {
        // Codex P2 on #989: JSON.stringify returns `undefined` (NOT a
        // string) for certain values without throwing — symbols, plain
        // functions, and any value whose `.toJSON()` returns undefined.
        // The previous implementation forwarded that `undefined` to the
        // host log. Coerce via String() in that case to preserve a
        // useful diagnostic shape.
        const out = JSON.stringify(value);
        return typeof out === "string" ? out : String(value);
    } catch {
        return String(value);
    }
}

function forward(tag: string, payload: ForwardedErrorPayload): void {
    try {
        const headline = `${tag} ${payload.name ?? "Error"}: ${payload.message}`;
        // Fire-and-forget — never let logging break the app
        invokeCommand("fe_log_structured", {
            level: "error",
            module: "uncaught",
            message: headline,
            data: {
                tag,
                name: payload.name,
                message: payload.message,
                stack: payload.stack,
                source: payload.source,
            },
        }).catch(() => {});
    } catch {
        // swallow
    }
}

function extractErrorFields(err: unknown): {
    name: string | null;
    message: string;
    stack: string | null;
} {
    if (err instanceof Error) {
        return {
            name: err.name ?? null,
            message: err.message ?? String(err),
            stack: err.stack ?? null,
        };
    }
    return {
        name: null,
        message: safeStringify(err),
        stack: null,
    };
}

export function initErrorForwarder(): void {
    if (initialized) return;
    initialized = true;

    window.addEventListener("error", (event: ErrorEvent) => {
        // Codex P2 on #989: when `event.error` is null/undefined (common
        // for cross-origin and resource/script errors), the previous
        // implementation passed `undefined` through `extractErrorFields`
        // which returned the literal string "undefined" via
        // `safeStringify(undefined)`. That string is truthy, so the
        // `fields.message || event.message` chain never reached the
        // ErrorEvent fallback — the host logged "undefined" instead of
        // the browser-provided error text. Short-circuit when there is
        // no Error object: build the payload from ErrorEvent fields
        // directly.
        if (event.error == null) {
            forward("[uncaught-error]", {
                name: null,
                message: event.message || "(no message)",
                stack: null,
                source: event.filename || null,
            });
            return;
        }
        const fields = extractErrorFields(event.error);
        forward("[uncaught-error]", {
            name: fields.name,
            message: fields.message || event.message || "(no message)",
            stack: fields.stack,
            source: event.filename || null,
        });
    });

    window.addEventListener("unhandledrejection", (event: PromiseRejectionEvent) => {
        const fields = extractErrorFields(event.reason);
        forward("[unhandled-rejection]", {
            name: fields.name,
            message: fields.message,
            stack: fields.stack,
            source: null,
        });
    });
}
