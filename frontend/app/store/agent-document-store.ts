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
import { markDispatch } from "../view/agent/virtualization/perf-probe";
import { update } from "./agent-document/reducer";
import {
    AgentDocumentCommand,
    AgentDocumentEvent,
    AgentDocumentState,
    type ReducerOptions,
    initialState,
} from "./agent-document/types";
import { type CommandSource, recordDispatch } from "./command-source";

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
    } else if (event.type === "tool-chunk-dropped") {
        // Real signal — chunk arrived before the matching ToolNode
        // existed in state (or the tool was already terminated).
        // Indicates an ordering bug, not a transient — keep loud.
        console.warn(
            `[agent-document-store] dropped chunk for ${blockId.slice(0, 7)} toolId=${event.toolId.slice(0, 14)} reason=${event.reason}`,
        );
    }
};

/**
 * Register a pane with the store. The slot owns the pane's reducer
 * state and writes through the setter on each mutation.
 *
 * Idempotent: re-registering a blockId resets its state to initialState
 * and rebinds the setter. (Useful for hot-reload scenarios.)
 *
 * @internal — production callers MUST use `registerPane` from
 * `agent-pane-registration.ts` so the pane is registered atomically
 * across BOTH stores (document + pane-state). Direct callers of this
 * function are limited to single-store unit tests (cascade-detection
 * scenarios that need a custom single-store projection). PR-3 of the
 * cascade follow-up sequence — see agent-pane-registration.ts for
 * rationale + Option A/B discussion.
 */
export function registerPane(blockId: string, setter: DocumentSetter): void {
    slots.set(blockId, { state: initialState(), setter });
}

/**
 * Release a pane's slot. Failure to release leaks a state cell —
 * every register MUST have a matching unregister.
 *
 * @internal — see `registerPane` above. Production code uses
 * `unregisterPane` from `agent-pane-registration.ts`.
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
    source: CommandSource = "system",
    opts?: ReducerOptions,
): AgentDocumentEvent[] {
    const slot = slots.get(blockId);
    if (!slot) {
        throw new Error(
            `[agent-document-store] dispatch for unregistered pane ${blockId.slice(0, 7)} (cmd=${command.type}). registerPane must be called synchronously in the component body.`,
        );
    }
    // Perf probe (task #40): times the FULL dispatch — reducer + the
    // setter push below — the exact layer the original virtualization
    // probe never measured (it only timed row DOM mount). No-op outside
    // dev (see markDispatch / isProbingEnabled).
    const stopDispatchTiming = markDispatch("document");
    const result = update(slot.state, command, Date.now(), opts);
    const prevNodes = slot.state.nodes;
    slot.state = result.state;
    // Push to atom only if nodes changed (referential equality).
    // Avoids no-op signal writes that would still schedule a Solid effect.
    if (slot.state.nodes !== prevNodes) {
        slot.setter(slot.state.nodes);
        // Cascade detection: docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md.
        // documentAtom subscribers can unmount the pane synchronously
        // during this setter; the slot then vanishes mid-dispatch and
        // the caller's next dispatch (often dispatchPane on a sibling
        // store) throws. Log so the trigger is identifiable.
        if (!slots.has(blockId)) {
            console.warn(
                `[agent-document-store] CASCADE_DETECTED: slot disposed mid-dispatch ` +
                `(cmd=${command.type}, blockId=${blockId.slice(0, 7)}, source=${source}). ` +
                `A documentAtom subscriber unmounted the pane during this dispatch. ` +
                `Subsequent dispatches in the same callback will throw.`,
            );
        }
    }
    for (const ev of result.events) eventSink(blockId, ev);
    recordDispatch({
        slice: "agent-document",
        key: blockId,
        command,
        events: result.events,
        source,
        at: Date.now(),
    });
    stopDispatchTiming();
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
    command: AgentDocumentCommand,
    source: CommandSource = "system",
    opts?: ReducerOptions,
): AgentDocumentEvent[] {
    if (!slots.has(blockId)) return [];
    return dispatch(blockId, command, source, opts);
}

/** Snapshot a pane's reducer state. Diagnostics + tests only. */
export function snapshot(blockId: string): AgentDocumentState | null {
    return slots.get(blockId)?.state ?? null;
}

/**
 * Read the reducer-maintained dedup index for a pane. Returns an empty
 * set if the pane isn't registered (e.g. very early during mount).
 * Issue #728 gap 4 — replaces the per-mount rebuild in useAgentStream.
 */
export function getNodeIdSet(blockId: string): Set<string> {
    return slots.get(blockId)?.state.nodeIdSet ?? new Set<string>();
}

/** Test/dev helper — wipe all slots. Never call in production. */
export function __resetAllSlots(): void {
    slots.clear();
}

// Re-export types for ergonomic imports at the call site.
export type { AgentDocumentCommand, AgentDocumentEvent, AgentDocumentState };
