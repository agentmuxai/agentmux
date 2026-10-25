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
 * else. The reactive read side of `nodes` is owned here too (A6 of
 * issue #1549 — it used to be a view-owned `documentAtom` signal whose
 * setter was handed in at registration).
 *
 * Pattern modeled on `launcher-event-store.ts`: single module,
 * in-memory mirror, audit-friendly.
 */

import { type Accessor, createSignal, type Setter } from "solid-js";

import type { DocumentNode } from "../view/agent/types";
import { markDispatch } from "../view/agent/virtualization/perf-probe";
import { RpcApi } from "./rpc-api";
import { TabRpcClient } from "./rpc-util";
import { update } from "./agent-document/reducer";
import {
    AgentDocumentCommand,
    AgentDocumentEvent,
    AgentDocumentState,
    type ReducerOptions,
    initialState,
} from "./agent-document/types";
import { type CommandSource, recordDispatch } from "./command-source";

/**
 * Push a final `docknodestatus` delta for every tool node an
 * `orphans-scrubbed` event resolved locally (SessionEnd/HistoryLoaded/
 * HistoryRestored/ScrubOrphanedInProgress) — otherwise `muxspect dock`'s
 * server-side snapshot cache never learns the node resolved and can keep
 * reporting it as STUCK? forever (reagentx P1, PR #2432). Fire-and-forget,
 * same as the streaming-path push in stream-flush-queue.ts's
 * pushDockNodeStatus — this is the single choke point every dispatch path
 * (streaming AND scrub) funnels through, so one push site here covers both
 * instead of touching each of the 5 scrub call sites individually.
 */
function pushResolvedDockNodes(blockId: string, events: AgentDocumentEvent[]) {
    for (const ev of events) {
        if (ev.type !== "orphans-scrubbed") continue;
        for (const n of ev.resolvedToolNodes) {
            void RpcApi.DockNodeStatusCommand(TabRpcClient, {
                blockid: blockId,
                node_id: n.id,
                tool_name: n.toolName,
                status: n.status,
                // reagent P1 on PR #2520: DockSnapshotCache::push_delta fully
                // overwrites a node's snapshot per push — omitting this would
                // silently blank an earlier `run_in_background: true` push
                // back to undefined right when a node resolves via scrub,
                // erasing the signal for exactly the orphaned/stuck-launch
                // class this field exists to flag.
                run_in_background: n.run_in_background,
            }).catch(() => {});
        }
    }
}

interface Slot {
    state: AgentDocumentState;
    /**
     * Reactive read side of `state.nodes`. Referential equality, same as
     * the view-owned signal it replaced: the reducer produces a fresh array
     * whenever the node list changes, so identity is the change signal.
     */
    nodes: Accessor<DocumentNode[]>;
    setNodes: Setter<DocumentNode[]>;
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
        // existed in state (or the tool was already terminated). Counted
        // and logged once per burst: a pane that mounts has up to 1024
        // persisted chunks replayed at it before its history has made the
        // tool nodes, and one console.warn (one IPC log line) per chunk
        // was ~1400 lines in 2 s on a tab tear-off.
        noteChunkDropped(blockId, event.toolId, event.reason);
    }
};

/** A burst of dropped chunks ends after this long without another one. */
const DROP_BURST_QUIET_MS = 1000;
/** Distinct (tool, reason) pairs a burst names; the rest are counted as "other". */
const DROP_BURST_MAX_TOOLS = 200;
/** Tools named in the summary line, busiest first. */
const DROP_SUMMARY_TOP = 3;

interface DropBurst {
    /** `${reason} ${toolId}` → chunks dropped. */
    byTool: Map<string, number>;
    total: number;
    /** Drops past `DROP_BURST_MAX_TOOLS` distinct pairs. */
    other: number;
    timer: ReturnType<typeof setTimeout> | null;
}

const dropBursts = new Map<string, DropBurst>();

function noteChunkDropped(blockId: string, toolId: string, reason: string): void {
    let burst = dropBursts.get(blockId);
    if (burst == null) {
        burst = { byTool: new Map(), total: 0, other: 0, timer: null };
        dropBursts.set(blockId, burst);
    }
    const key = `${reason} ${toolId}`;
    const seen = burst.byTool.get(key);
    if (seen != null) burst.byTool.set(key, seen + 1);
    else if (burst.byTool.size < DROP_BURST_MAX_TOOLS) burst.byTool.set(key, 1);
    else burst.other++;
    burst.total++;
    if (burst.timer != null) clearTimeout(burst.timer);
    burst.timer = setTimeout(() => flushDropBurst(blockId), DROP_BURST_QUIET_MS);
}

/** Logs the pane's burst of dropped chunks as ONE line, and ends the burst. */
function flushDropBurst(blockId: string): void {
    const burst = dropBursts.get(blockId);
    if (burst == null) return;
    dropBursts.delete(blockId);
    if (burst.timer != null) clearTimeout(burst.timer);
    const pane = blockId.slice(0, 7);
    if (burst.total === 1) {
        const [reason, toolId] = [...burst.byTool.keys()][0].split(" ");
        console.warn(`[agent-document-store] dropped chunk for ${pane} toolId=${toolId.slice(0, 14)} reason=${reason}`);
        return;
    }
    const byReason = new Map<string, number>();
    for (const [key, n] of burst.byTool) {
        const reason = key.slice(0, key.indexOf(" "));
        byReason.set(reason, (byReason.get(reason) ?? 0) + n);
    }
    if (burst.other > 0) byReason.set("other", burst.other);
    const reasons = [...byReason].map(([r, n]) => `${r}=${n}`).join(" ");
    const busiest = [...burst.byTool]
        .sort((a, b) => b[1] - a[1])
        .slice(0, DROP_SUMMARY_TOP)
        .map(([key, n]) => `${key.slice(key.indexOf(" ") + 1, key.indexOf(" ") + 15)}x${n}`)
        .join(" ");
    const tools = burst.byTool.size + (burst.other > 0 ? "+" : "");
    console.warn(
        `[agent-document-store] dropped ${burst.total} chunks for ${tools} tool ids in ${pane} (${reasons}); busiest: ${busiest}`,
    );
}

/**
 * Register a pane with the store. The slot owns the pane's reducer
 * state and its reactive `nodes` accessor.
 *
 * Idempotent: re-registering a blockId resets its state to initialState
 * and creates a fresh accessor. (Useful for hot-reload scenarios.)
 *
 * @internal — production callers MUST use `registerPane` from
 * `agent-pane-registration.ts` so the pane is registered atomically
 * across BOTH stores (document + pane-state). Direct callers of this
 * function are limited to single-store unit tests. PR-3 of the cascade
 * follow-up sequence — see agent-pane-registration.ts for rationale +
 * Option A/B discussion.
 */
export function registerPane(blockId: string): void {
    const state = initialState();
    const [nodes, setNodes] = createSignal<DocumentNode[]>(state.nodes);
    slots.set(blockId, { state, nodes, setNodes });
}

/**
 * Reactive accessor for a pane's document nodes, or `null` if the pane is
 * not registered. Production code reaches this through
 * `AgentPaneModel.document`; this export is for tests and diagnostics.
 */
export function documentNodes(blockId: string): Accessor<DocumentNode[]> | null {
    return slots.get(blockId)?.nodes ?? null;
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
    flushDropBurst(blockId);
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
    // signal publish below — the exact layer the original virtualization
    // probe never measured (it only timed row DOM mount). No-op outside
    // dev (see markDispatch / isProbingEnabled).
    const stopDispatchTiming = markDispatch("document");
    const result = update(slot.state, command, Date.now(), opts);
    const prevNodes = slot.state.nodes;
    slot.state = result.state;
    // Publish only if nodes changed (referential equality). Avoids no-op
    // signal writes that would still schedule a Solid effect. Thunk form
    // so Solid never mistakes the array for an updater function.
    if (slot.state.nodes !== prevNodes) {
        const nodes = slot.state.nodes;
        slot.setNodes(() => nodes);
        // Cascade detection: docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md.
        // A `nodes` subscriber can unmount the pane synchronously during
        // this publish; the slot then vanishes mid-dispatch and the
        // caller's next dispatch (often dispatchPane on a sibling store)
        // throws. Log so the trigger is identifiable.
        if (!slots.has(blockId)) {
            console.warn(
                `[agent-document-store] CASCADE_DETECTED: slot disposed mid-dispatch ` +
                `(cmd=${command.type}, blockId=${blockId.slice(0, 7)}, source=${source}). ` +
                `A document-nodes subscriber unmounted the pane during this dispatch. ` +
                `Subsequent dispatches in the same callback will throw.`,
            );
        }
    }
    for (const ev of result.events) eventSink(blockId, ev);
    pushResolvedDockNodes(blockId, result.events);
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
    for (const burst of dropBursts.values()) if (burst.timer != null) clearTimeout(burst.timer);
    dropBursts.clear();
}

// Re-export types for ergonomic imports at the call site.
export type { AgentDocumentCommand, AgentDocumentEvent, AgentDocumentState };
