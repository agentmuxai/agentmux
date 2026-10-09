// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Turns you didn't start that finished while you weren't looking: another
 * agent's jekt, a service notice, a scheduled run, a finished background
 * task. Watched app-wide from every block's turn ledger (`agentturn`), so a
 * pane that isn't mounted still collects them. A pane shows a dot while it
 * has any, and a one-line summary when you come back to it; looking clears
 * them. Attention follows need: a quiet external turn marks the pane, it
 * doesn't interrupt (OS notifications for these are off by default, see the
 * notify router's `external_turns`).
 *
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §5.1, §5.3.
 */

import { createSignal } from "solid-js";
import { parseTurnLedger, type TurnLedger, type TurnTrigger } from "./agent-pane-state/turn-ledger";

/** At most this many triggers are kept per pane; `count` keeps going. */
const KEEP_TRIGGERS = 10;

export interface UnseenTurns {
    count: number;
    /** What started them, oldest first (capped). */
    triggers: TurnTrigger[];
    lastEndedAt: number;
}

const [unseen, setUnseen] = createSignal<Readonly<Record<string, UnseenTurns>>>({});
/** Turns already counted, so a turn's end published twice counts once. */
const counted = new Set<string>();

/** The pane's unseen external turns, or null (reactive). */
export function unseenTurnsFor(blockId: string): UnseenTurns | null {
    return unseen()[blockId] ?? null;
}

function isExternal(t: TurnTrigger | null): t is TurnTrigger {
    return !!t && (t.kind === "agent" || t.kind === "service" || t.kind === "schedule" || t.kind === "task");
}

/**
 * A ledger arrived. Counts it when it reports an external turn over and the
 * user wasn't looking at its pane (`watching`: the window had focus with that
 * block focused). Returns whether it counted.
 */
export function noteTurnLedger(blockId: string, ledger: TurnLedger, watching: boolean): boolean {
    if (ledger.end !== "completed" || !isExternal(ledger.trigger) || watching) return false;
    const key = `${blockId}:${ledger.turnId}`;
    if (counted.has(key)) return false;
    counted.add(key);
    if (counted.size > 5_000) counted.clear();
    const prev = unseen()[blockId];
    const triggers = [...(prev?.triggers ?? []), ledger.trigger].slice(-KEEP_TRIGGERS);
    setUnseen({
        ...unseen(),
        [blockId]: { count: (prev?.count ?? 0) + 1, triggers, lastEndedAt: ledger.endedAtMs ?? ledger.lastPassEndedAtMs ?? Date.now() },
    });
    return true;
}

/** The user has looked: the pane's unseen turns, now cleared (null if none). */
export function markTurnsSeen(blockId: string): UnseenTurns | null {
    const prev = unseen()[blockId];
    if (!prev) return null;
    const next = { ...unseen() };
    delete next[blockId];
    setUnseen(next);
    return prev;
}

/** Tests only. */
export function resetTurnAwareness(): void {
    setUnseen({});
    counted.clear();
}

const NOUN: Record<string, [string, string]> = {
    agent: ["jekt", "jekts"],
    service: ["notice", "notices"],
    schedule: ["scheduled run", "scheduled runs"],
    task: ["finished task", "finished tasks"],
};

/**
 * The return summary: "While you were away: 3 turns · 2 jekts (AgentX,
 * Korp) · 1 finished task". Senders are named for jekts and notices, up to
 * three.
 */
export function awaySummary(u: UnseenTurns): string {
    const byKind = new Map<string, TurnTrigger[]>();
    for (const t of u.triggers) byKind.set(t.kind, [...(byKind.get(t.kind) ?? []), t]);
    const parts = [...byKind].map(([kind, ts]) => {
        const [one, many] = NOUN[kind] ?? ["turn", "turns"];
        const who = kind === "agent" || kind === "service" ? [...new Set(ts.map((t) => t.from).filter((f): f is string => !!f))] : [];
        const names = who.length ? ` (${who.slice(0, 3).join(", ")}${who.length > 3 ? ", …" : ""})` : "";
        return `${ts.length} ${ts.length === 1 ? one : many}${names}`;
    });
    const head = `While you were away: ${u.count} ${u.count === 1 ? "turn" : "turns"}`;
    return parts.length ? `${head} · ${parts.join(" · ")}` : head;
}

let installed = false;

/**
 * Watch every block's turn ledger, app-wide. `watching(blockId)` says whether
 * the user is looking at that block right now.
 */
export function installTurnAwareness(deps: {
    subscribe: (handler: (blockId: string, data: unknown) => void) => () => void;
    watching: (blockId: string) => boolean;
}): () => void {
    if (installed) return () => {};
    installed = true;
    const unsub = deps.subscribe((blockId, data) => {
        const ledger = parseTurnLedger(data);
        if (ledger) noteTurnLedger(blockId, ledger, deps.watching(blockId));
    });
    return () => {
        installed = false;
        unsub();
    };
}
