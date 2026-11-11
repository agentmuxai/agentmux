// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a promise was rejected with, in words, for a log line.
 *
 * A library that loads an image, a script or a file often rejects with the element's `error` event itself
 * (`img.onerror = reject`). Stringified, that is `{"isTrusted":true}`, which names nothing: so an event is
 * described by its type and by what it happened on.
 */

const MAX_SOURCE = 160;

type EventSource = { src?: unknown; href?: unknown; url?: unknown; currentSrc?: unknown };

function sourceOf(target: EventSource): string {
    const from = [target.currentSrc, target.src, target.href, target.url].find((v) => typeof v === "string" && v !== "");
    if (typeof from !== "string") return "";
    // A data URL is the whole file: its kind is enough.
    const shown = from.startsWith("data:") ? from.slice(0, from.indexOf(",") + 1 || 32) + "…" : from;
    return shown.length > MAX_SOURCE ? `${shown.slice(0, MAX_SOURCE)}…` : shown;
}

/** "error event on img https://example.com/favicon.ico": the event's type, its target and where that loads from. */
export function describeEvent(ev: Event): string {
    const target = ev.target as (EventSource & { tagName?: unknown; constructor?: { name?: string } }) | null;
    if (target == null) return `${ev.type} event`;
    const what = typeof target.tagName === "string" ? target.tagName.toLowerCase() : (target.constructor?.name ?? "object");
    const from = sourceOf(target);
    return `${ev.type} event on ${what}${from ? ` ${from}` : ""}`;
}

/** Whether `reason` is a DOM event (where there is a DOM). */
export function isEvent(reason: unknown): reason is Event {
    return typeof Event !== "undefined" && reason instanceof Event;
}

/** A rejection reason as one line: an Error's message, an event described, anything else as text. */
export function describeRejection(reason: unknown): string {
    if (reason instanceof Error) return reason.message || reason.name;
    if (isEvent(reason)) return describeEvent(reason);
    if (typeof reason === "string") return reason;
    try {
        return JSON.stringify(reason) ?? String(reason);
    } catch {
        return String(reason);
    }
}
