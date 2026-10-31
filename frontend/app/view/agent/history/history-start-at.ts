// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Open this agent's history at that message": a request written on the reader's block meta
// (open-history-tab.ts) and applied, then cleared, by the reader (AgentHistoryView). A request on the meta
// is a one-shot: a reader reopened later starts at the end, as it always did.

/** Block-meta key of a pending request to scroll the reader to a position. */
export const HISTORY_AT_META_KEY = "agent:historyAt";

/** A position in the agent's record: a time, or a byte (the more exact), in a zone when it is not the current conversation. */
export interface HistoryAt {
    at_ms?: number;
    at_byte?: number;
    /** An archive's zone id, absent for the current conversation. */
    zone?: string;
    /** Distinguishes two requests for the same position, so the second is applied too. */
    n: number;
}

export function parseHistoryAt(value: unknown): HistoryAt | null {
    if (!value || typeof value !== "object") return null;
    const v = value as Record<string, unknown>;
    const num = (x: unknown): number | undefined => (typeof x === "number" && Number.isFinite(x) ? x : undefined);
    const at_ms = num(v.at_ms);
    const at_byte = num(v.at_byte);
    if (at_ms === undefined && at_byte === undefined) return null;
    return {
        ...(at_ms !== undefined ? { at_ms } : {}),
        ...(at_byte !== undefined ? { at_byte } : {}),
        ...(typeof v.zone === "string" && v.zone ? { zone: v.zone } : {}),
        n: num(v.n) ?? 0,
    };
}

/** The request to store for a position the Audit holds. The byte, when there is one, locates exactly; the time otherwise. */
export function makeHistoryAt(at: { at_ms?: number; at_byte?: number; zone?: string }): HistoryAt {
    return { ...at, n: Date.now() };
}
