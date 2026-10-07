// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Console log pipe: monkey-patches console.log/warn/error/debug/info
// to forward all messages to the Rust host. Original console behavior is
// preserved.
//
// Lines are batched: they collect for up to LOG_FLUSH_MS and go to the host in
// one `fe_log_batch` request, with at most one in flight. Sending each line as
// its own request let a chatty moment (every browser pane logging every
// favicon event during a window drag, ~750 lines a second) fill the
// connections every other IPC call shares, and browser panes trailed a resize
// by ~90 ms waiting for one
// (docs/analysis/ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md §8).
//
// Usage: call initLogPipe() once at startup, before any other code.

import { invokeCommand } from "@/app/platform/ipc";

const LEVELS = ["log", "warn", "error", "debug", "info"] as const;
type LogLevel = (typeof LEVELS)[number];

export interface LogEntry {
    level: string;
    module: string;
    message: string;
    data: unknown;
}

/** How long lines collect before a batch is sent. */
export const LOG_FLUSH_MS = 100;
/** Lines held at most; past this the oldest are dropped and counted. */
export const LOG_BUFFER_MAX = 2000;

export type SendLogBatch = (entries: LogEntry[]) => Promise<unknown>;

export interface LogBatcher {
    push(entry: LogEntry): void;
}

export function createLogBatcher(
    send: SendLogBatch,
    schedule: (cb: () => void, ms: number) => void = (cb, ms) => void setTimeout(cb, ms)
): LogBatcher {
    let buffer: LogEntry[] = [];
    let dropped = 0;
    let scheduled = false;
    let inFlight = false;

    const flush = () => {
        scheduled = false;
        if (inFlight || (buffer.length === 0 && dropped === 0)) return;
        const batch = buffer;
        buffer = [];
        if (dropped > 0) {
            batch.unshift({
                level: "warn",
                module: "log-pipe",
                message: `[log-pipe] ${dropped} console lines dropped: more than ${LOG_BUFFER_MAX} were waiting`,
                data: null,
            });
            dropped = 0;
        }
        inFlight = true;
        send(batch)
            .catch(() => {})
            .finally(() => {
                inFlight = false;
                if (buffer.length > 0 || dropped > 0) arm();
            });
    };
    const arm = () => {
        if (scheduled) return;
        scheduled = true;
        schedule(flush, LOG_FLUSH_MS);
    };

    return {
        push(entry) {
            buffer.push(entry);
            if (buffer.length > LOG_BUFFER_MAX) {
                dropped += buffer.length - LOG_BUFFER_MAX;
                buffer.splice(0, buffer.length - LOG_BUFFER_MAX);
            }
            if (!inFlight) arm();
        },
    };
}

/** The host's batch command, falling back to one request per line on a host without it. */
export function hostSendLogBatch(): SendLogBatch {
    let batchUnsupported = false;
    const perLine = (entries: LogEntry[]) =>
        Promise.all(entries.map((e) => invokeCommand("fe_log_structured", { ...e }).catch(() => {})));
    return async (entries) => {
        if (batchUnsupported) return perLine(entries);
        try {
            await invokeCommand("fe_log_batch", { entries });
        } catch (e) {
            if (!/unknown command/i.test(String(e))) throw e;
            batchUnsupported = true;
            return perLine(entries);
        }
    };
}

let initialized = false;

export function initLogPipe() {
    if (initialized) return;
    initialized = true;
    const batcher = createLogBatcher(hostSendLogBatch());

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

                // Never let logging break the app
                batcher.push({ level: level === "log" ? "info" : level, module: "console", message: msg, data: null });
            } catch {
                // swallow
            }
        };
    }
}
