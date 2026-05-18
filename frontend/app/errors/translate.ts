// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Wire-format ↔ user-facing translator for `AgentMuxError`.
 *
 * The backend serializes errors as:
 *   `{ "code": "AMX-IO-001", "message": "...", "details": { ... } }`
 *
 * `translateError(raw)` accepts either the wire object directly OR a
 * JavaScript `Error` whose `message` field happens to be the JSON
 * representation (this is what `RpcClient` produces today). It always
 * returns a renderable `{ code, title, message, retry?, rawMessage }`.
 *
 * Un-recognised codes fall through to a clean "Something went wrong"
 * frame so legacy backend strings are never displayed raw.
 */

import { ERROR_CATALOG } from "./catalog";

export interface TranslatedError {
    /** The stable AMX code, or `AMX-LEGACY` for un-typed errors. */
    code: string;
    /** Headline for the banner. */
    title: string;
    /** Sentence-case body. */
    message: string;
    /** Optional recovery hint. */
    retry?: string;
    /** The original backend message — kept for the Details disclosure. */
    rawMessage: string;
}

interface WireError {
    code: string;
    message?: string;
    details?: Record<string, unknown>;
}

function asWireError(value: unknown): WireError | null {
    if (value && typeof value === "object" && "code" in value) {
        const code = (value as { code: unknown }).code;
        if (typeof code === "string") {
            return value as WireError;
        }
    }
    return null;
}

function tryParseJson(text: string): unknown {
    if (!text || (text[0] !== "{" && text[0] !== "[")) return null;
    try {
        return JSON.parse(text);
    } catch {
        return null;
    }
}

export function translateError(raw: unknown): TranslatedError {
    // Path 1: backend already returned the typed wire object.
    const direct = asWireError(raw);
    if (direct) {
        return renderEntry(direct);
    }

    // Path 2: `RpcClient` wraps the wire object inside an Error whose
    // `.message` is the JSON string. Probe it.
    if (raw instanceof Error) {
        const parsed = tryParseJson(raw.message);
        const wire = asWireError(parsed);
        if (wire) return renderEntry(wire);
        // Plain JS Error with a free-text message — render as legacy.
        return legacyEntry(raw.message);
    }

    // Path 3: arbitrary value (string, number, undefined) — coerce.
    return legacyEntry(typeof raw === "string" ? raw : String(raw ?? "unknown error"));
}

function renderEntry(wire: WireError): TranslatedError {
    const entry = ERROR_CATALOG[wire.code];
    const details = wire.details ?? {};
    const rawMessage = wire.message ?? "";
    if (!entry) {
        return {
            code: wire.code,
            title: "Something went wrong",
            message: rawMessage || `An unexpected error occurred (${wire.code}).`,
            rawMessage,
        };
    }
    return {
        code: wire.code,
        title: entry.title,
        message: entry.message(details),
        retry: entry.retry,
        rawMessage,
    };
}

function legacyEntry(message: string): TranslatedError {
    const entry = ERROR_CATALOG["AMX-LEGACY"];
    return {
        code: "AMX-LEGACY",
        title: entry?.title ?? "Something went wrong",
        message: entry ? entry.message({ message }) : message,
        rawMessage: message,
    };
}
