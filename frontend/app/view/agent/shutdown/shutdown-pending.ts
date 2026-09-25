// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A shutdown waiting on the user's override
 * (docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5, events §12.2): srv
 * publishes `agent:shutdown-pending` when something other than the user asks
 * to shut this agent down, and `agent:shutdown-pending-cleared` when the user
 * kept it or the window ran out.
 */

export const EVENT_SHUTDOWN_PENDING = "agent:shutdown-pending";
export const EVENT_SHUTDOWN_PENDING_CLEARED = "agent:shutdown-pending-cleared";
/** The second chime, this long before the deadline. */
export const LAST_CALL_MS = 5_000;

export interface PendingShutdown {
    request_id: string;
    block_id: string;
    by: string;
    via: string;
    reason: string;
    deadline_ms: number;
}

export function parsePending(data: unknown): PendingShutdown | null {
    const d = data as Partial<PendingShutdown> | null;
    if (!d || typeof d.request_id !== "string" || typeof d.deadline_ms !== "number") return null;
    return {
        request_id: d.request_id,
        block_id: typeof d.block_id === "string" ? d.block_id : "",
        by: typeof d.by === "string" ? d.by : "",
        via: typeof d.via === "string" ? d.via : "",
        reason: typeof d.reason === "string" ? d.reason : "",
        deadline_ms: d.deadline_ms,
    };
}

/** The request id a `-cleared` event is for. */
export function clearedRequestId(data: unknown): string | null {
    const id = (data as { request_id?: unknown } | null)?.request_id;
    return typeof id === "string" ? id : null;
}

/**
 * What a pane that opens mid-countdown should show, from the persisted last
 * `pending` and last `cleared` events: the pending one, unless it was
 * cleared or its deadline has passed.
 */
export function replayPending(pending: unknown, cleared: unknown, now: number): PendingShutdown | null {
    const p = parsePending(pending);
    if (!p || p.deadline_ms <= now) return null;
    return clearedRequestId(cleared) === p.request_id ? null : p;
}

/** Whole seconds left, never negative. */
export function secondsLeft(p: PendingShutdown, now: number): number {
    return Math.max(0, Math.ceil((p.deadline_ms - now) / 1000));
}

/**
 * "Korp asked to shut down Camper: <reason>." — or "Camper asked to shut
 * itself down" for its own `QuitSelf` from a turn the user didn't start.
 */
export function pendingTitle(p: PendingShutdown, agentId: string, agentName: string): string {
    const self = p.by.toLowerCase() === agentId.toLowerCase() || p.by.toLowerCase() === agentName.toLowerCase();
    const who = self ? `${agentName} asked to shut itself down` : `${p.by || "Something"} asked to shut down ${agentName}`;
    return p.reason ? `${who}: ${p.reason}` : who;
}
