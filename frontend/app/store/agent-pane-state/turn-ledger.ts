// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The turn as the user sees it: from the agent leaving idle to its return
 * there with nothing queued, over however many CLI passes that took. srv keeps
 * it (`TurnLedger` in crates/srv/src/backend/blockcontroller/health.rs) and
 * publishes it as the persisted `agentturn` event; the pane only reads it.
 *
 * Within a turn, the reducer's `turnTokens` and `TurnEnd` stay per pass.
 * These helpers lay the pass over the turn: the working row's clock and
 * counter run from the turn's start, and its "Worked" line reports the turn.
 *
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §3, §4.
 */

import type { TurnTokens } from "@/app/view/agent/types";
import { turnOutputTokens } from "./turn-contribution";

export type TurnLedgerOrigin = "user" | "automated" | "system";

export interface TurnLedger {
    turnId: number;
    /** srv bumps it on every change; within one turn, a lower one is older. */
    seq: number;
    /** What started the turn; null when its first pass was unlabelled. */
    origin: TurnLedgerOrigin | null;
    startedAtMs: number;
    /** CLI passes so far, the running one included. */
    passes: number;
    /** A pass is running now. */
    active: boolean;
    /** Inputs that arrived after the turn started. */
    inputs: number;
    /** Passes whose `result` figures are in the sums below. */
    countedPasses: number;
    outputTokens: number;
    costUsd: number;
    /** Model calls (`result.num_turns`). */
    steps: number;
    durationApiMs: number;
    lastPassEndedAtMs: number | null;
    /** Between passes, while a next pass is expected: over if none starts by then. */
    settleUntilMs: number | null;
    endedAtMs: number | null;
    end: "completed" | "exited" | null;
}

function num(v: unknown): number | null {
    return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/** Parse an `agentturn` event's data; null if it isn't one. */
export function parseTurnLedger(data: unknown): TurnLedger | null {
    if (!data || typeof data !== "object") return null;
    const d = data as Record<string, unknown>;
    const turnId = num(d.turn_id);
    const startedAtMs = num(d.started_at_ms);
    if (turnId == null || startedAtMs == null || typeof d.active !== "boolean") return null;
    const origin = d.origin === "user" || d.origin === "automated" || d.origin === "system" ? d.origin : null;
    const end = d.end === "completed" || d.end === "exited" ? d.end : null;
    return {
        turnId,
        seq: num(d.seq) ?? 0,
        origin,
        startedAtMs,
        passes: num(d.passes) ?? 1,
        active: d.active,
        inputs: num(d.inputs) ?? 0,
        countedPasses: num(d.counted_passes) ?? 0,
        outputTokens: num(d.output_tokens) ?? 0,
        costUsd: num(d.cost_usd) ?? 0,
        steps: num(d.steps) ?? 0,
        durationApiMs: num(d.duration_api_ms) ?? 0,
        lastPassEndedAtMs: num(d.last_pass_ended_at_ms),
        settleUntilMs: num(d.settle_until_ms),
        endedAtMs: num(d.ended_at_ms),
        end,
    };
}

/** Whether `next` replaces `current`: a later turn, or a later state of the
 *  same turn. srv publishes outside its lock, so an older state can land
 *  after a newer one. */
export function isNewerLedger(next: TurnLedger, current: TurnLedger | null | undefined): boolean {
    if (!current) return true;
    if (next.turnId !== current.turnId) return next.turnId > current.turnId;
    return next.seq >= current.seq;
}

/** Between two passes of one turn, waiting for the next: no pass running, not
 *  ended, inside the settle window. */
export function turnSettling(l: TurnLedger | null | undefined, nowMs: number): boolean {
    return !!l && !l.active && l.endedAtMs == null && l.settleUntilMs != null && nowMs <= l.settleUntilMs;
}

/** The turn is still going: a pass runs, or the next one is expected. */
export function turnOpen(l: TurnLedger | null | undefined, nowMs: number): boolean {
    return !!l && (l.active || turnSettling(l, nowMs));
}

/** How long after a turn ended srv still lets a held message join it
 *  (`HELD_FLUSH_JOIN_MS` in health.rs). */
export const HELD_FLUSH_JOIN_MS = 10_000;

/** The turn a held message being flushed now should join: the one still open,
 *  or the one that only just ended. The message was held because a turn was
 *  running, and the flush follows that turn's end at once. */
export function turnToJoin(l: TurnLedger | null | undefined, nowMs: number): number | undefined {
    if (!l) return undefined;
    if (turnOpen(l, nowMs)) return l.turnId;
    const endedAt = turnEndedAt(l, nowMs);
    return endedAt != null && nowMs - endedAt <= HELD_FLUSH_JOIN_MS ? l.turnId : undefined;
}

/** When the turn ended: its end, or the last pass's end once the settle window lapsed. */
export function turnEndedAt(l: TurnLedger, nowMs: number): number | null {
    if (l.endedAtMs != null) return l.endedAtMs;
    if (turnOpen(l, nowMs)) return null;
    return l.lastPassEndedAtMs;
}

/**
 * The turn's output so far, for the live counter: the counted passes' exact
 * figures, plus the running pass's live count while srv hasn't counted it.
 * `turnTokens` carries the turn and pass it was stamped with (`TokensIn`), so
 * tokens from a pass srv already counted are never added twice. Outside an
 * open turn, it is the pass's own count, as before the ledger existed.
 */
export function turnLiveOutput(
    l: TurnLedger | null | undefined,
    t: TurnTokens | null | undefined,
    nowMs: number,
): number | undefined {
    const live = t ? turnOutputTokens(t) : undefined;
    if (!l || !turnOpen(l, nowMs)) return live;
    const uncounted =
        live != null &&
        (t?.ledgerTurnId == null ? l.countedPasses === 0 : t.ledgerTurnId === l.turnId && (t.ledgerPass ?? 0) > l.countedPasses);
    if (l.countedPasses === 0) return uncounted ? live : undefined;
    return l.outputTokens + (uncounted ? (live ?? 0) : 0);
}
