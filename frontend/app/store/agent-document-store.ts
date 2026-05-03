// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent document store — Option B from
 * docs/specs/agent-pane-document-reducer-2026-05-03.md.
 *
 * Single store-level module that owns the per-pane state cells
 * (`Map<blockId, AgentDocumentState>`) and dispatches commands through
 * the pure reducer. Every mutation to an agent pane's message list
 * flows through this module — no direct setDocument calls anywhere
 * else. The pane's `documentAtom` becomes a write-only projection.
 *
 * Pattern modeled on `launcher-event-reducer.ts`: single module,
 * in-memory mirror, atom projection, audit-friendly.
 */

import type { DocumentNode } from "../view/agent/types";
import { update } from "./agent-document/reducer";
import {
    AgentDocumentCommand,
    AgentDocumentEvent,
    AgentDocumentState,
    initialState,
} from "./agent-document/types";

/** A pane's projection setter — typically the SignalPair[1] from createAgentAtoms. */
type DocumentSetter = (nodes: DocumentNode[]) => void;

interface Slot {
    state: AgentDocumentState;
    setter: DocumentSetter;
}

const slots = new Map<string, Slot>();

/**
 * Optional global event sink. The agent-view diagnostics panel can
 * hook this to surface the reducer event stream. v1: console.warn for
 * suppressed truncates and dropped updates.
 */
type EventSink = (blockId: string, event: AgentDocumentEvent) => void;
let eventSink: EventSink = (blockId, event) => {
    if (event.type === "truncate-suppressed") {
        console.warn(
            `[agent-document-store] suppressed truncate for ${blockId.slice(0, 7)}: reason=${event.reason} activeForMs=${event.activeForMs} nodeCount=${event.nodeCount}`,
        );
    } else if (event.type === "stream-flushed" && event.updateDropped > 0) {
        console.warn(
            `[agent-document-store] dropped ${event.updateDropped} stream updates for unknown ids in ${blockId.slice(0, 7)}`,
        );
    }
};

/** Override the event sink (tests / diagnostics panel). */
export function setEventSink(sink: EventSink): void {
    eventSink = sink;
}

/**
 * Register a pane with the store. Call from the agent-view component
 * onMount, passing the pane's documentAtom setter. The slot owns the
 * pane's reducer state and writes through the setter on each mutation.
 *
 * Idempotent: re-registering a blockId resets its state to initialState
 * and rebinds the setter. (Useful for hot-reload scenarios.)
 */
export function registerPane(blockId: string, setter: DocumentSetter): void {
    slots.set(blockId, { state: initialState(), setter });
}

/**
 * Release a pane's slot. Call from agent-view component onCleanup.
 * Failure to release leaks a state cell — every register MUST have a
 * matching unregister.
 */
export function unregisterPane(blockId: string): void {
    slots.delete(blockId);
}

/**
 * Apply a command to a pane's reducer state. Returns the audit events
 * the command produced (typically zero or one event).
 *
 * If the blockId isn't registered the dispatch throws — silent drop is
 * dangerous because it would mean a real reducer command (like
 * `SessionStart` or an early `StreamFlush`) was lost, and the next
 * `StreamTruncate` would then be honored against an uninitialized state
 * cell, reintroducing the wipe class this whole module exists to
 * prevent. Convention: panes call `registerPane` synchronously during
 * component-body execution, before any hook can dispatch from its own
 * `onMount`.
 *
 * The throw is structurally safe — Solid components don't unmount mid-
 * dispatch (cleanup is synchronous and always runs after children).
 */
export function dispatch(
    blockId: string,
    command: AgentDocumentCommand,
): AgentDocumentEvent[] {
    const slot = slots.get(blockId);
    if (!slot) {
        throw new Error(
            `[agent-document-store] dispatch for unregistered pane ${blockId.slice(0, 7)} (cmd=${command.type}). registerPane must be called synchronously in the component body.`,
        );
    }
    const result = update(slot.state, command);
    const prevNodes = slot.state.nodes;
    slot.state = result.state;
    // Push to atom only if nodes changed (referential equality).
    // Avoids no-op signal writes that would still schedule a Solid effect.
    if (slot.state.nodes !== prevNodes) {
        slot.setter(slot.state.nodes);
    }
    for (const ev of result.events) eventSink(blockId, ev);
    return result.events;
}

/** Snapshot a pane's reducer state. Diagnostics + tests only. */
export function snapshot(blockId: string): AgentDocumentState | null {
    return slots.get(blockId)?.state ?? null;
}

/** Test/dev helper — wipe all slots. Never call in production. */
export function __resetAllSlots(): void {
    slots.clear();
}

// Re-export types for ergonomic imports at the call site.
export type { AgentDocumentCommand, AgentDocumentEvent, AgentDocumentState };
