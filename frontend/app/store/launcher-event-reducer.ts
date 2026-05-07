// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Frontend reducer dispatch + projection layer for launcher typed events.
//
// Refactored in PR-B (slice #6 of the frontend reducer roadmap, 2026-05-03)
// to follow the conventions established in
// docs/specs/frontend-reducer-conventions-2026-05-03.md. The pure reducer
// + types live in `./launcher-event/`; this file owns:
//   - the in-memory state cell (single global slice)
//   - the SolidJS effect that subscribes to the launcherEvent signal
//   - the projection layer (writes derived state into global atoms)
//   - the echo-loop guard (`applyingRemote`)
//   - the public API: startLauncherEventReducer, seedKnownEntriesFromSnapshot,
//     isApplyingRemoteEvent, isInstanceLabel
//
// Behavior is unchanged from the prior in-place dispatch — see
// `launcher-event/reducer.test.ts` for the 19 backfilled tests.
//
// History context (kept for reviewer):
// - Phase B.7.3.1 (PR #602): scaffolding only — logged events, no atom mutation.
// - Phase B.7.3.2 (PR #603): typed events became authoritative for InstancePanel.
// - Phase B.7.3.3 (PR #604): bespoke `window-instances-changed` channel retired.
// - PR-B (this PR): pure refactor + tests. No behavior change.
//
// See docs/specs/SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29.md.

import { createEffect } from "solid-js";

import { launcherEvent, launcherEventVersion } from "@/util/launcher-events";
import {
    setOpenWindowEntriesAtom,
    setOpenWindowLabelsAtom,
    setWindowCountAtom,
    type WindowEntry,
} from "@/app/store/global";

import { update } from "./launcher-event/reducer";
import {
    initialState,
    isInstanceLabel as isInstanceLabelFromTypes,
    LauncherEventCommand,
    LauncherEventReducerEvent,
    LauncherEventState,
} from "./launcher-event/types";
import { recordDispatch } from "./command-source";

// Re-export the filter so existing callers (app-init.ts) don't need to
// chase the new module path. The filter is the source of truth for
// what the InstancePanel surfaces — if the two filters ever diverge,
// reagent caught it on PR #603 and will catch it again.
export const isInstanceLabel = isInstanceLabelFromTypes;

// ── State cell ─────────────────────────────────────────────────────────

let state: LauncherEventState = initialState();

// ── Echo-loop guard ────────────────────────────────────────────────────

let applyingRemote = false;

/**
 * True while the reducer is mid-apply for a launcher event. Future
 * renderer-emitted commands should check this and skip re-emission to
 * avoid echo loops with the launcher.
 *
 * Currently no commands flow through this bridge (commands still take
 * the host IPC HTTP path), so the flag is forward-compatibility
 * scaffolding — same as before the refactor.
 */
export function isApplyingRemoteEvent(): boolean {
    return applyingRemote;
}

// ── Dispatch + projection ──────────────────────────────────────────────

function dispatch(command: LauncherEventCommand): LauncherEventReducerEvent[] {
    const prev = state;
    const result = update(prev, command);
    state = result.state;
    if (state.instances !== prev.instances) project();
    for (const ev of result.events) onAuditEvent(ev);
    // Source is always "system" — this slice mirrors upstream events
    // (launcher channel) and the snapshot seed; no user-driven path.
    recordDispatch({
        slice: "launcher-event",
        key: null,
        command,
        events: result.events,
        source: "system",
        at: Date.now(),
    });
    return result.events;
}

/**
 * Project derived state to the global atoms. Called only when
 * `instances` changed (referential equality), avoiding redundant atom
 * writes that would re-run subscribers.
 */
function project(): void {
    setOpenWindowLabelsAtom(state.instances.map((e) => e.label));
    setOpenWindowEntriesAtom([...state.instances]);
    setWindowCountAtom(state.instances.length);
}

/**
 * Audit-event sink. Notable variants are logged to console (matching
 * pre-refactor behavior); the rest are silent. The diagnostics-panel
 * surface will hook this once PR-C ships.
 */
function onAuditEvent(event: LauncherEventReducerEvent): void {
    if (event.type === "drift-detected") {
        console.warn("[launcher-event] drift", event.raw);
    } else if (event.type === "saga-event-observed") {
        console.info("[launcher-event]", event.eventName, event.raw);
    }
}

// ── Public API ─────────────────────────────────────────────────────────

/**
 * Seed `knownEntries` from the init RPC snapshot. Called once from
 * `app-init.ts::initInstanceTracking` after `listWindowInstances`
 * returns. The reducer's ApplySeed arm preserves existing entries
 * (codex P1 #603) and skips tombstoned labels (codex P2 #603).
 */
export function seedKnownEntriesFromSnapshot(
    entries: ReadonlyArray<WindowEntry>,
): void {
    dispatch({ type: "ApplySeed", entries });
}

/**
 * Reconcile knownEntries against a fresh `listWindowInstances` snapshot.
 * Differs from `seedKnownEntriesFromSnapshot` (`ApplySeed`) in that it
 * REPLACES the known set wholesale — labels absent from the snapshot
 * are removed from the panel.
 *
 * Use for periodic refresh paths (e.g. InstancePanel reopens in
 * `task dev` mode where the launcher doesn't push WindowClosed
 * events). Don't use at boot — `ApplySeed` is the right boot path
 * because it's additive against typed events that may have raced
 * the snapshot fetch (codex P1 #603).
 */
export function reconcileKnownEntriesFromSnapshot(
    entries: ReadonlyArray<WindowEntry>,
): void {
    dispatch({ type: "ReconcileFromSnapshot", entries });
}

let started = false;

/**
 * Start the reducer effect. Idempotent. Called once per renderer
 * after `initWaveWrap` so global state is ready before the first
 * apply touches atoms.
 */
export function startLauncherEventReducer(): void {
    if (started) return;
    started = true;

    createEffect(() => {
        const evt = launcherEvent();
        // Read version too so SolidJS tracks the version signal as a
        // dependency — guarantees the effect re-runs even when two
        // consecutive events have referentially-equal payloads (e.g.
        // a same-shape OffMonitor drift firing twice).
        launcherEventVersion();
        if (!evt) return;

        applyingRemote = true;
        try {
            dispatch({ type: "ApplyEvent", event: evt });
        } finally {
            applyingRemote = false;
        }
    });
}

// ── Test/diagnostics helpers ───────────────────────────────────────────

/**
 * Snapshot for tests + future diagnostics. Avoid in production code
 * paths — bypasses the projection layer.
 */
export function __snapshot(): LauncherEventState {
    return state;
}

/** Test-only: reset the state cell. Never call in production. */
export function __resetState(): void {
    state = initialState();
    started = false;
}
