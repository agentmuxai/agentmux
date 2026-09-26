// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One text format for every copyable error, and one place that logs an
//! error without losing its stack — `SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md`
//! §4.1 and §6.1. Surfaces pass fields; they never format the text
//! themselves.

import { redactSecrets } from "./redact";

export interface ErrorReportFields {
    /** The surface's own short title, e.g. "Agent failure", "Can't reconnect". */
    title: string;
    /** The human-readable error message, if distinct from the title. */
    message?: string;
    /** AMX code / HTTP status / exit code / signal — whichever applies, already formatted (e.g. "HTTP 401", "exit 1", "signal 9"). */
    code?: string;
    /** "<surface> · pane <short block id> · <view type or provider>". */
    where?: string;
    /** Defaults to now. */
    when?: Date;
    /** Full detail text — stderr tail, stack, render trail. Never truncated to what's on screen; original line breaks kept. */
    details?: string;
}

function pad(n: number, width = 2): string {
    return String(n).padStart(width, "0");
}

/** ISO-8601 in the local timezone (not UTC `Z`) — §4.1's "When:" line. */
export function toIsoLocalString(date: Date): string {
    const offsetMin = -date.getTimezoneOffset();
    const sign = offsetMin >= 0 ? "+" : "-";
    const abs = Math.abs(offsetMin);
    const offset = `${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`;
    return (
        `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
        `T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}${offset}`
    );
}

/**
 * Builds the one clipboard text format every copy action shares (§4.1).
 * Lines with nothing to say are omitted. Redacts the *whole* report before
 * any caller-side truncation (the redact-then-cap rule, §4.5) — callers
 * never need to redact their own fields.
 */
export function formatErrorReport(fields: ErrorReportFields): string {
    const lines: string[] = [`AgentMux error: ${fields.title}`];
    if (fields.message) lines.push(fields.message);
    if (fields.code) lines.push(`Code: ${fields.code}`);
    if (fields.where) lines.push(`Where: ${fields.where}`);
    lines.push(`When: ${toIsoLocalString(fields.when ?? new Date())}`);
    if (fields.details) {
        lines.push("", "Details:", fields.details);
    }
    return redactSecrets(lines.join("\n"));
}

export interface DescribedError {
    name: string;
    message: string;
    stack?: string;
    cause?: string;
}

/**
 * `{name, message, stack, cause}` from anything a `catch` clause might hand
 * back — an `Error`, a string, or an arbitrary thrown value. Fixes §6.1's
 * "startup errors lose their detail": logging `describeError(e)` instead of
 * the bare error object, and passing its text instead of `String(e)`, keeps
 * the stack instead of serializing to `{}`.
 */
export function describeError(e: unknown): DescribedError {
    if (e instanceof Error) {
        const cause = (e as { cause?: unknown }).cause;
        return {
            name: e.name || "Error",
            message: e.message || String(e),
            stack: e.stack,
            cause: cause === undefined ? undefined : describeError(cause).message,
        };
    }
    if (typeof e === "string") {
        return { name: "Error", message: e };
    }
    if (e && typeof e === "object") {
        try {
            return { name: "Error", message: JSON.stringify(e) };
        } catch {
            // fall through to the generic String(e) below
        }
    }
    return { name: "Error", message: String(e) };
}

/** Flattens a `describeError` result to text, for surfaces that take a
 * plain string (e.g. `showStartupError`'s "Technical details" pane). */
export function formatDescribedError(d: DescribedError): string {
    const lines = [`${d.name}: ${d.message}`];
    if (d.cause) lines.push(`Caused by: ${d.cause}`);
    if (d.stack) lines.push("", d.stack);
    return lines.join("\n");
}
