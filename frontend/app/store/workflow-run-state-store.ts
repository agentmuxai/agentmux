// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Workflow run state store — slice #10 of the frontend reducer roadmap
 * and PR 4 of Phase 1.5 (`docs/specs/SPEC_UNIFIED_AGENT_TYPES_2026_05_13.md`
 * §6 row 4). Closes the "workflows-model.ts is not a reducer" drift
 * item by externalizing per-run state from `WorkflowsViewModel` and
 * routing every dispatch through the `recordDispatch` audit ring.
 *
 * Pattern matches slice #9 (`browser-pane-state-store.ts`) — same slot
 * lifecycle (`registerPane` / `dispatch` / `unregisterPane`), same
 * "throw on unregistered dispatch" rule, same projection-on-change
 * discipline.
 *
 * Key choices specific to this slice:
 *   - `blockResults` is a `Record<string, AgentBlockResult>`. The
 *     projector calls `proj.blockResults(snapshot)` with the WHOLE
 *     map only when the object identity changed — the reducer
 *     allocates a fresh map on every BlockDone/BlockError/Backfill,
 *     so reference equality is the right cheap check.
 *   - `runId` doubles as "is there a live run" (`""` = none). The
 *     view binds the inspector's result panel to `blockResults` not
 *     to `runId`, so a backfill into an already-terminal run still
 *     surfaces results.
 */

import { type CommandSource, recordDispatch } from "./command-source";
import { update } from "./workflow-run-state/reducer";
import {
    AgentBlockResult,
    initialState,
    WorkflowRunCommand,
    WorkflowRunEvent,
    WorkflowRunState,
    WorkflowRunStatus,
} from "./workflow-run-state/types";

/**
 * Setters the slot writes into when reducer state changes. The view
 * (`WorkflowsViewModel`) owns the underlying SolidJS signals and
 * passes just the setters in via `registerPane`. Readers continue
 * using the model's existing accessors — only writes flow through
 * the slot.
 */
export interface WorkflowRunProjections {
    closed: (next: boolean) => void;
    runId: (next: string) => void;
    workflowId: (next: string) => void;
    status: (next: WorkflowRunStatus) => void;
    blockResults: (next: Record<string, AgentBlockResult>) => void;
    output: (next: string) => void;
    error: (next: string) => void;
}

interface Slot {
    state: WorkflowRunState;
    proj: WorkflowRunProjections;
}

const slots = new Map<string, Slot>();

type EventSink = (blockId: string, event: WorkflowRunEvent) => void;
let eventSink: EventSink = (_blockId, _event) => {
    // Default no-op sink. Tests can override via `setEventSink`.
};

export function setEventSink(sink: EventSink): void {
    eventSink = sink;
}

/**
 * Register a pane. Call SYNCHRONOUSLY from the model's constructor so
 * subsequent IPC handlers see a registered slot. Re-registering a
 * blockId resets the state cell — useful for hot-reload paths where
 * a fresh model may claim the same blockId.
 */
export function registerPane(
    blockId: string,
    proj: WorkflowRunProjections,
): void {
    slots.set(blockId, { state: initialState(), proj });
}

export function unregisterPane(blockId: string): void {
    slots.delete(blockId);
}

/**
 * Apply a command. Throws on unregistered blockId — silent drops
 * defeat the reducer's audit value.
 *
 * The `closed` invariant — every command after `Disposed` becomes a
 * no-op that emits `post-close-command-dropped` — is enforced inside
 * the pure reducer.
 */
export function dispatch(
    blockId: string,
    command: WorkflowRunCommand,
    source: CommandSource = "system",
): WorkflowRunEvent[] {
    const slot = slots.get(blockId);
    if (!slot) {
        throw new Error(
            `[workflow-run-state] dispatch for unregistered pane ${blockId.slice(0, 7)} (cmd=${command.type}). registerPane must be called synchronously in the WorkflowsViewModel constructor.`,
        );
    }
    const prev = slot.state;
    const result = update(prev, command);
    slot.state = result.state;

    if (slot.state.closed !== prev.closed) slot.proj.closed(slot.state.closed);
    if (slot.state.runId !== prev.runId) slot.proj.runId(slot.state.runId);
    if (slot.state.workflowId !== prev.workflowId)
        slot.proj.workflowId(slot.state.workflowId);
    if (slot.state.status !== prev.status) slot.proj.status(slot.state.status);
    if (slot.state.blockResults !== prev.blockResults)
        slot.proj.blockResults(slot.state.blockResults);
    if (slot.state.output !== prev.output) slot.proj.output(slot.state.output);
    if (slot.state.error !== prev.error) slot.proj.error(slot.state.error);

    for (const ev of result.events) eventSink(blockId, ev);

    recordDispatch({
        slice: "workflow-run-state",
        key: blockId,
        command,
        events: result.events,
        source,
        at: Date.now(),
    });

    return result.events;
}

/** Snapshot — diagnostics + tests only. */
export function snapshot(blockId: string): WorkflowRunState | null {
    return slots.get(blockId)?.state ?? null;
}

/** Test/dev helper — clears every slot. */
export function __resetAllSlots(): void {
    slots.clear();
}

export type {
    AgentBlockResult,
    WorkflowRunCommand,
    WorkflowRunEvent,
    WorkflowRunState,
    WorkflowRunStatus,
};
