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
} from "./agent-pane-state/types";
import { type CommandSource, recordDispatch } from "./command-source";

/**
 * The set of projection setters the slot writes to. Each one corresponds
 * to a pre-existing per-pane Solid signal in `createAgentAtoms`. Readers
 * keep using the existing accessors; only writes are routed through this
 * store.
 */
export interface AgentPaneProjections {
    streaming: (next: StreamingState) => void;
    sessionStats: (next: SessionStats | null) => void;
    currentTool: (next: string | null) => void;
    turnTokens: (next: TurnTokens | null) => void;
    turnActive: (next: boolean) => void;
    stopping: (next: boolean) => void;
    pending: (next: PendingMessage[]) => void;
    /** Init phase — drives the "Loading history…" overlay (issue #728 gap 1). */
    initPhase?: (next: InitPhase) => void;
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
    if (slot.state.streaming !== prev.streaming) slot.proj.streaming(slot.state.streaming);
    if (slot.state.sessionStats !== prev.sessionStats) slot.proj.sessionStats(slot.state.sessionStats);
    if (slot.state.currentTool !== prev.currentTool) slot.proj.currentTool(slot.state.currentTool);
    if (slot.state.turnTokens !== prev.turnTokens) slot.proj.turnTokens(slot.state.turnTokens);
    if (slot.state.turnActive !== prev.turnActive) slot.proj.turnActive(slot.state.turnActive);
    if (slot.state.stopping !== prev.stopping) slot.proj.stopping(slot.state.stopping);
    if (slot.state.pending !== prev.pending) slot.proj.pending(slot.state.pending);
    if (slot.state.initPhase !== prev.initPhase) slot.proj.initPhase?.(slot.state.initPhase);

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

/** Snapshot — diagnostics + tests only. */
export function snapshot(blockId: string): AgentPaneState | null {
    return slots.get(blockId)?.state ?? null;
}

/** Test/dev helper. */
export function __resetAllSlots(): void {
    slots.clear();
}

export type { AgentPaneCommand, AgentPaneEvent, AgentPaneState };
