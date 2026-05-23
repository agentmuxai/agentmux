// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent pane state store — slice #4 of the frontend reducer roadmap.
 * Bundles the per-pane lifecycle/turn/tool/tokens/stop/pending atoms.
 *
 * Pattern matches agent-document-store.ts: per-blockId slot, atoms as
 * write-only projections, throw on unregistered dispatch. Conventions
 * §4–§5 (frontend-reducer-conventions-2026-05-03.md).
 */

import type { PendingMessage } from "../view/agent/state";
import type {
    SessionStats,
    StreamingState,
    TurnTokens,
} from "../view/agent/types";
import { update } from "./agent-pane-state/reducer";
import {
    AgentPaneCommand,
    AgentPaneEvent,
    AgentPaneState,
    type InitPhase,
    initialState,
    type TurnPhase,
} from "./agent-pane-state/types";
import { type CommandSource, recordDispatch } from "./command-source";

/**
 * The set of projection setters the slot writes to. Each one corresponds
 * to a pre-existing per-pane Solid signal in `createAgentAtoms`. Readers
 * keep using the existing accessors; only writes are routed through this
 * store.
 *
 * PR G dropped the `turnActive` and `stopping` setters — the legacy
 * fields they backed are gone. The view binds its "working" animation
 * to `workingFromPhase(turnPhase)` and its "Stopping…" label to
 * `turnPhase.kind === "Interrupting"`.
 */
export interface AgentPaneProjections {
    streaming: (next: StreamingState) => void;
    sessionStats: (next: SessionStats | null) => void;
    currentTool: (next: string | null) => void;
    turnTokens: (next: TurnTokens | null) => void;
    pending: (next: PendingMessage[]) => void;
    /** Init phase — drives the "Loading history…" overlay (issue #728 gap 1). */
    initPhase?: (next: InitPhase) => void;
    /**
     * Single-source-of-truth turn phase. Since PR G this is the only
     * working/stopping signal the view binds to (via
     * `workingFromPhase(turnPhase)` and `turnPhase.kind === "Interrupting"`).
     * Optional so existing callers (and the cascade-store test's no-op
     * projection) keep compiling.
     */
    turnPhase?: (next: TurnPhase) => void;
}

interface Slot {
    state: AgentPaneState;
    proj: AgentPaneProjections;
}

const slots = new Map<string, Slot>();

type EventSink = (blockId: string, event: AgentPaneEvent) => void;
let eventSink: EventSink = (blockId, event) => {
    if (event.type === "turn-start-suppressed") {
        console.warn(
            `[agent-pane-state] turn-start suppressed for ${blockId.slice(0, 7)}: ${event.reason}`,
        );
    }
};

export function setEventSink(sink: EventSink): void {
    eventSink = sink;
}

/**
 * Register a pane. Call SYNCHRONOUSLY from the component body, before
 * any hook can dispatch. Re-registering a blockId resets the state cell
 * to initialState (useful for hot-reload).
 */
export function registerPane(
    blockId: string,
    agentId: string,
    proj: AgentPaneProjections,
): void {
    slots.set(blockId, { state: initialState(agentId), proj });
}

export function unregisterPane(blockId: string): void {
    slots.delete(blockId);
}

/**
 * Apply a command. Throws on unregistered blockId — silent drops would
 * defeat the point of the reducer (same rule as agent-document-store).
 */
export function dispatch(
    blockId: string,
    command: AgentPaneCommand,
    source: CommandSource = "system",
): AgentPaneEvent[] {
    const slot = slots.get(blockId);
    if (!slot) {
        throw new Error(
            `[agent-pane-state] dispatch for unregistered pane ${blockId.slice(0, 7)} (cmd=${command.type}). registerPane must be called synchronously in the component body.`,
        );
    }
    const prev = slot.state;
    const result = update(prev, command);
    slot.state = result.state;

    // Project changes — only call setters for fields that actually
    // changed (referential equality). Avoids redundant signal writes.
    // Per-setter cascade detection: docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md.
    // A reactive subscriber on the atom backing one of these setters can
    // synchronously unmount the pane (call `unregisterPane`) inside the
    // setter call. Capture which setter triggers the dispose — the next
    // dispatch in the caller's frame will throw and the log line below
    // pinpoints the cause.
    let cascadeSetter: string | null = null;
    const proj = <T>(name: string, prev: T, next: T, set: ((v: T) => void) | undefined): void => {
        if (prev === next) return;
        set?.(next);
        if (cascadeSetter == null && !slots.has(blockId)) cascadeSetter = name;
    };
    proj("streaming", prev.streaming, slot.state.streaming, slot.proj.streaming);
    proj("sessionStats", prev.sessionStats, slot.state.sessionStats, slot.proj.sessionStats);
    proj("currentTool", prev.currentTool, slot.state.currentTool, slot.proj.currentTool);
    proj("turnTokens", prev.turnTokens, slot.state.turnTokens, slot.proj.turnTokens);
    proj("pending", prev.pending, slot.state.pending, slot.proj.pending);
    proj("initPhase", prev.initPhase, slot.state.initPhase, slot.proj.initPhase);
    proj("turnPhase", prev.turnPhase, slot.state.turnPhase, slot.proj.turnPhase);

    if (cascadeSetter != null) {
        console.warn(
            `[agent-pane-state] CASCADE_DETECTED: '${cascadeSetter}' setter disposed pane mid-dispatch ` +
            `(cmd=${command.type}, blockId=${blockId.slice(0, 7)}, source=${source}). ` +
            `A reactive subscriber on the '${cascadeSetter}' atom unmounted the pane during dispatch. ` +
            `Subsequent dispatches in the same callback will throw.`,
        );
    }

    for (const ev of result.events) eventSink(blockId, ev);
    recordDispatch({
        slice: "agent-pane-state",
        key: blockId,
        command,
        events: result.events,
        source,
        at: Date.now(),
    });
    return result.events;
}

/**
 * Soft-dispatch variant. Returns an empty event array if the slot is
 * already gone, instead of throwing. Use ONLY from async contexts
 * (RAF / setTimeout / setInterval / await continuations / subscription
 * handlers) where a normal dispatch can race against the pane's
 * onCleanup unregistering the slot — see
 * docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md §6.1 option B.
 *
 * Synchronous component-body dispatches MUST continue to use `dispatch`
 * — a missing slot there is a registration-order bug and the throw is
 * the right signal.
 */
export function dispatchIfRegistered(
    blockId: string,
    command: AgentPaneCommand,
    source: CommandSource = "system",
): AgentPaneEvent[] {
    if (!slots.has(blockId)) return [];
    return dispatch(blockId, command, source);
}

/** Snapshot — diagnostics + tests only. */
export function snapshot(blockId: string): AgentPaneState | null {
    return slots.get(blockId)?.state ?? null;
}

/** Test/dev helper. */
export function __resetAllSlots(): void {
    slots.clear();
}

export type { AgentPaneCommand, AgentPaneEvent, AgentPaneState };
