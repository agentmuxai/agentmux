// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A shutdown waiting on the user's override
 * (docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5, events §12.2): srv
 * publishes `agent:shutdown-pending` when something other than the user asks
 * to shut this agent down, and `agent:shutdown-pending-cleared` when the user
 * kept it or the window ran out.
 */

import { WpsEvent } from "@/app/store/mps-events";
import type { PendingView } from "@/types/rpc/PendingView";

export const EVENT_SHUTDOWN_PENDING = WpsEvent.AgentShutdownPending;
export const EVENT_SHUTDOWN_PENDING_CLEARED = WpsEvent.AgentShutdownPendingCleared;
/** The second chime, this long before the deadline. */
export const LAST_CALL_MS = 5_000;

/** What happens when the window runs out (`PendingView.effect`). */
export type ShutdownEffect = PendingView["effect"];

export interface PendingShutdown {
    request_id: string;
    block_id: string;
    by: string;
    via: string;
    /** The asker's reason, already cleaned by srv: never the raw text. */
    reason: string;
    effect: ShutdownEffect;
    deadline_ms: number;
    /** The other agents closing in the same request (`Stop agents` over several); 0 for one. */
    others: number;
}

export function parsePending(data: unknown, block = ""): PendingShutdown | null {
    const d = data as (Partial<PendingShutdown> & { blocks?: unknown }) | null;
    if (!d || typeof d.request_id !== "string" || typeof d.deadline_ms !== "number") return null;
    return {
        request_id: d.request_id,
        block_id: typeof d.block_id === "string" ? d.block_id : "",
        by: typeof d.by === "string" ? d.by : "",
        via: typeof d.via === "string" ? d.via : "",
        reason: typeof d.reason === "string" ? d.reason : "",
        effect: d.effect === "stop" || d.effect === "close_pane" ? d.effect : "close_tab",
        deadline_ms: d.deadline_ms,
        others: othersOf(d, block),
    };
}

/** How many agents besides the one in `block` the request is closing: its `blocks` list less this pane's. */
function othersOf(d: { blocks?: unknown }, block: string): number {
    if (!Array.isArray(d.blocks)) return 0;
    const ids = d.blocks.map((b) => (b as { block_id?: unknown } | null)?.block_id);
    return ids.filter((id) => typeof id === "string" && id !== block).length;
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
export function replayPending(pending: unknown, cleared: unknown, now: number, block = ""): PendingShutdown | null {
    const p = parsePending(pending, block);
    if (!p || p.deadline_ms <= now) return null;
    return clearedRequestId(cleared) === p.request_id ? null : p;
}

/** Whole seconds left, never negative. */
export function secondsLeft(p: PendingShutdown, now: number): number {
    return Math.max(0, Math.ceil((p.deadline_ms - now) / 1000));
}

/**
 * "Korp asked to close Camper: <reason>." — or "Camper asked to close
 * itself" for its own `QuitSelf` from a turn the user didn't start. A close
 * ends the agent and takes its tab; only the internal stop keeps the pane,
 * and says "stop". The reason is shown as text and is already cleaned.
 */
export function pendingTitle(p: PendingShutdown, agentId: string, agentName: string): string {
    const self = p.by.toLowerCase() === agentId.toLowerCase() || p.by.toLowerCase() === agentName.toLowerCase();
    const verb = p.effect === "stop" ? "stop" : "close";
    const also = p.others > 0 ? ` and ${p.others} other${p.others === 1 ? "" : "s"}` : "";
    const who = self && !also ? `${agentName} asked to ${verb} itself` : `${p.by || "Something"} asked to ${verb} ${agentName}${also}`;
    return p.reason ? `${who}: ${p.reason}` : who;
}

/** "Closing in 12 s" / "Stopping in 12 s". */
export function countdownLine(p: PendingShutdown, seconds: number): string {
    return `${p.effect === "stop" ? "Stopping" : "Closing"} in ${seconds} s.`;
}
