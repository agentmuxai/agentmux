// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for launcher typed events. Slice #6 in
 * docs/specs/frontend-reducer-architecture-2026-05-03.md (PR-B
 * convergence). Pattern matches the host/launcher Rust reducers.
 *
 * Behavior verbatim from the prior in-place dispatch in
 * launcher-event-reducer.ts — this is a refactor PR, not a redesign.
 * The seed-vs-close race fixes (codex P1/P2 #603, #604) are preserved
 * exactly; tests verify each scenario.
 */

import type { LauncherEvent } from "@/util/launcher-events";
import {
    isInstanceLabel,
    LauncherEventCommand,
    LauncherEventReducerEvent,
    LauncherEventState,
    ReducerResult,
    WindowEntry,
} from "./types";

export function update(
    state: LauncherEventState,
    command: LauncherEventCommand,
): ReducerResult {
    switch (command.type) {
        case "ApplyEvent":
            return applyEvent(state, command.event);
        case "ApplySeed":
            return applySeed(state, command.entries);
        case "ReconcileFromSnapshot":
            return reconcileFromSnapshot(state, command.entries);
    }
}

// ── ApplyEvent dispatcher ──────────────────────────────────────────────

function applyEvent(state: LauncherEventState, evt: LauncherEvent): ReducerResult {
    switch (evt.event) {
        case "window_opened":
            return handleWindowOpened(state, String(evt.label ?? ""));
        case "window_closed":
            return handleWindowClosed(state, String(evt.label ?? ""), "window-closed");
        case "window_instance_assigned":
            return handleWindowInstanceAssigned(state, String(evt.label ?? ""));
        case "window_instance_released":
            return handleWindowClosed(
                state,
                String(evt.label ?? ""),
                "window-instance-released",
            );
        case "backend_window_id_registered":
            return handleBackendWindowIdRegistered(
                state,
                String(evt.label ?? ""),
                typeof evt.window_id === "string" ? evt.window_id : null,
            );
        case "backend_window_id_unregistered":
            return handleBackendWindowIdUnregistered(state, String(evt.label ?? ""));
        case "hwnd_drift_detected":
            return {
                state,
                events: [{ type: "drift-detected", eventName: evt.event, raw: evt }],
            };
        case "corrective_window_move":
        case "host_should_quit":
            return {
                state,
                events: [{ type: "saga-event-observed", eventName: evt.event, raw: evt }],
            };
        default:
            return {
                state,
                events: [{ type: "unknown-variant-ignored", eventName: (evt as any).event }],
            };
    }
}

// ── Per-event handlers ────────────────────────────────────────────────

function handleWindowOpened(state: LauncherEventState, label: string): ReducerResult {
    if (!label || !isInstanceLabel(label)) {
        return filteredOut(state, label, "non-instance-label");
    }
    // Preserve existing windowId if a BackendWindowIdRegistered arrived
    // before WindowOpened (rare, but launcher event ordering between
    // separate Commands isn't synchronized w.r.t. renderer apply
    // order — the per-pipe ordering guarantee is the host → renderer
    // hop, not host's emission order across distinct host-side mutations).
    const existing = state.knownEntries.get(label);
    const preservedWindowId = existing?.windowId != null;
    const next = new Map(state.knownEntries);
    next.set(label, { label, windowId: existing?.windowId ?? null });
    return {
        state: withDerived(state, next, state.closedBeforeSeed),
        events: [{ type: "window-opened", label, preservedWindowId }],
    };
}

function handleWindowClosed(
    state: LauncherEventState,
    label: string,
    eventLabel: "window-closed" | "window-instance-released",
): ReducerResult {
    if (!label) {
        // Empty label — match prior behavior: silent return.
        return { state, events: [] };
    }
    const wasPresent = state.knownEntries.has(label);
    let nextKnown: Map<string, WindowEntry> = new Map(state.knownEntries);
    if (wasPresent) nextKnown.delete(label);

    let nextClosedBeforeSeed = state.closedBeforeSeed;
    let tombstoned = false;
    if (!state.seedHasHappened && state.closedBeforeSeed) {
        // Pre-seed: record a tombstone so seed skips this label even
        // if it appears in the snapshot (codex P2 #603). The delete
        // above handles the case where WindowOpened arrived first
        // pre-seed — and if it did delete an existing entry, atoms
        // must recompute here too, otherwise the ghost row persists
        // until seed runs (or forever if seed fails). (codex P2 #604.)
        nextClosedBeforeSeed = new Set(state.closedBeforeSeed);
        (nextClosedBeforeSeed as Set<string>).add(label);
        tombstoned = true;
    }

    const event = (
        eventLabel === "window-closed"
            ? { type: "window-closed", label, tombstoned, deletedFromKnown: wasPresent }
            : {
                  type: "window-instance-released",
                  label,
                  tombstoned,
                  deletedFromKnown: wasPresent,
              }
    ) as LauncherEventReducerEvent;

    // Three branches:
    //  1. neither knownEntries nor tombstones changed → full no-op (return
    //     same state reference per conventions §3)
    //  2. tombstones changed but knownEntries unchanged → preserve instances
    //     reference so the projection layer doesn't re-write atoms (codex
    //     P2 PR #684 — was rebuilding `instances` array unnecessarily,
    //     causing redundant Solid subscriber re-runs)
    //  3. knownEntries changed → re-derive instances
    if (!wasPresent && nextClosedBeforeSeed === state.closedBeforeSeed) {
        return { state, events: [event] };
    }
    if (!wasPresent) {
        // Tombstone-only change — preserve instances reference.
        return {
            state: { ...state, closedBeforeSeed: nextClosedBeforeSeed },
            events: [event],
        };
    }
    return {
        state: withDerived(state, nextKnown, nextClosedBeforeSeed),
        events: [event],
    };
}

function handleWindowInstanceAssigned(
    state: LauncherEventState,
    label: string,
): ReducerResult {
    if (!label || !isInstanceLabel(label)) {
        return filteredOut(state, label, "non-instance-label");
    }
    if (state.knownEntries.has(label)) {
        return {
            state,
            events: [{ type: "window-instance-assigned", label, createdMissing: false }],
        };
    }
    const next = new Map(state.knownEntries);
    next.set(label, { label, windowId: null });
    return {
        state: withDerived(state, next, state.closedBeforeSeed),
        events: [{ type: "window-instance-assigned", label, createdMissing: true }],
    };
}

function handleBackendWindowIdRegistered(
    state: LauncherEventState,
    label: string,
    windowId: string | null,
): ReducerResult {
    if (!label || !isInstanceLabel(label)) {
        return filteredOut(state, label, "non-instance-label");
    }
    const existing = state.knownEntries.get(label);
    const changed = !existing || existing.windowId !== windowId;
    if (existing && !changed) {
        // Skip recompute if windowId actually didn't change AND the
        // entry was already present — avoids a redundant atom write
        // (SolidJS would treat a referentially-new array as a change
        // and re-run subscribers).
        return {
            state,
            events: [{ type: "backend-window-id-registered", label, windowId, changed: false }],
        };
    }
    const next = new Map(state.knownEntries);
    next.set(label, { label, windowId });
    return {
        state: withDerived(state, next, state.closedBeforeSeed),
        events: [{ type: "backend-window-id-registered", label, windowId, changed: true }],
    };
}

function handleBackendWindowIdUnregistered(
    state: LauncherEventState,
    label: string,
): ReducerResult {
    if (!label || !isInstanceLabel(label)) {
        return filteredOut(state, label, "non-instance-label");
    }
    const existing = state.knownEntries.get(label);
    if (!existing) return { state, events: [] };
    if (existing.windowId === null) return { state, events: [] };
    const next = new Map(state.knownEntries);
    next.set(label, { label, windowId: null });
    return {
        state: withDerived(state, next, state.closedBeforeSeed),
        events: [{ type: "backend-window-id-unregistered", label }],
    };
}

// ── ApplySeed ──────────────────────────────────────────────────────────

function applySeed(
    state: LauncherEventState,
    entries: ReadonlyArray<WindowEntry>,
): ReducerResult {
    // Important — does NOT clobber existing entries (codex P1 #603). The
    // reducer effect is started earlier than initInstanceTracking, so
    // typed events can arrive between snapshot-fetch and this seed call.
    // Any entry already in knownEntries was written by a typed event with
    // newer information than the snapshot's view of the same label;
    // overwriting would roll the windowId back to a stale null.
    const next = new Map(state.knownEntries);
    let added = 0;
    let tombstonesSkipped = 0;
    for (const e of entries) {
        if (!isInstanceLabel(e.label)) continue;
        if (next.has(e.label)) continue;
        // Pre-seed close tombstone — label closed between snapshot fetch
        // and seed; skip the re-add (codex P2 #603).
        if (state.closedBeforeSeed?.has(e.label)) {
            tombstonesSkipped++;
            continue;
        }
        next.set(e.label, { label: e.label, windowId: e.windowId });
        added++;
    }
    return {
        state: {
            knownEntries: next,
            seedHasHappened: true,
            closedBeforeSeed: null,
            instances: deriveInstances(next),
        },
        events: [{ type: "seeded", addedCount: added, tombstonesSkipped }],
    };
}

// ── ReconcileFromSnapshot ──────────────────────────────────────────────

function reconcileFromSnapshot(
    state: LauncherEventState,
    entries: ReadonlyArray<WindowEntry>,
): ReducerResult {
    // Wholesale REPLACE knownEntries with the fresh snapshot. Adds new
    // labels (like ApplySeed) AND removes labels absent from the
    // snapshot (unlike ApplySeed). Used for steady-state refresh in
    // dev mode where the launcher doesn't push close events. Only
    // accepts instance labels — non-instance labels in the snapshot
    // are filtered the same as in ApplySeed.
    //
    // No tombstone handling: by the time reconcile runs, the seed
    // phase is over (`seedHasHappened: true`); pre-seed tombstones
    // are no longer relevant.
    const next = new Map<string, WindowEntry>();
    for (const e of entries) {
        if (!isInstanceLabel(e.label)) continue;
        next.set(e.label, { label: e.label, windowId: e.windowId });
    }
    let added = 0;
    for (const label of next.keys()) {
        if (!state.knownEntries.has(label)) added++;
    }
    let removed = 0;
    for (const label of state.knownEntries.keys()) {
        if (!next.has(label)) removed++;
    }
    return {
        state: {
            knownEntries: next,
            seedHasHappened: state.seedHasHappened,
            closedBeforeSeed: state.closedBeforeSeed,
            instances: deriveInstances(next),
        },
        events: [
            {
                type: "reconciled",
                addedCount: added,
                removedCount: removed,
                totalAfter: next.size,
            },
        ],
    };
}

// ── Helpers ────────────────────────────────────────────────────────────

function filteredOut(
    state: LauncherEventState,
    label: string,
    reason: string,
): ReducerResult {
    return { state, events: [{ type: "filtered-out", label, reason }] };
}

/**
 * Build a new state with the given knownEntries / closedBeforeSeed and
 * a freshly-derived `instances` array. Centralized so the derive logic
 * lives in one place.
 */
function withDerived(
    state: LauncherEventState,
    knownEntries: ReadonlyMap<string, WindowEntry>,
    closedBeforeSeed: ReadonlySet<string> | null,
): LauncherEventState {
    return {
        knownEntries,
        seedHasHappened: state.seedHasHappened,
        closedBeforeSeed,
        instances: deriveInstances(knownEntries),
    };
}

/**
 * Derived view: filter by `isInstanceLabel`, sort with "main" pinned
 * first then alphabetical. Without sorting, panel rows shuffle across
 * recomputes (Map iteration is insertion-ordered, but the InstancePanel
 * uses array index for naming and special-cases "main").
 */
function deriveInstances(
    knownEntries: ReadonlyMap<string, WindowEntry>,
): ReadonlyArray<WindowEntry> {
    const filtered: WindowEntry[] = [];
    for (const e of knownEntries.values()) {
        if (isInstanceLabel(e.label)) filtered.push(e);
    }
    filtered.sort((a, b) => {
        if (a.label === "main") return -1;
        if (b.label === "main") return 1;
        return a.label.localeCompare(b.label);
    });
    return filtered;
}
