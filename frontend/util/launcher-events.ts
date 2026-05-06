// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.7.3.1 — renderer-side subscriber for launcher typed events.
// Phase E.6 — multi-source dispatcher with version tracking + saga buffering.
//
// Mirrors `agentmux_common::ipc::Event` on the wire. The host's CEF
// JS bridge (`agentmux-cef/src/launcher_event_bridge.rs`) calls
// `window.__agentmux_launcher_event(<json>)` once per top-level
// renderer per launcher event.
//
// Phase B.7.3.3 — typed events are the SOLE path for InstancePanel
// state. The bespoke `window-instances-changed` channel and its 4
// sync emit sites in the host are gone.
//
// Phase E.6 — dispatcher backed by the shared `PerSourceTracker`.
// Same version-monotonicity + saga-buffer semantics as the srv pipe.
// See `util/event-buffer.ts` for the contract.
//
// See `docs/specs/SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29.md`.

import { createSignal } from "solid-js";
import { PerSourceTracker, type EventCallback, type VersionedEvent, type PerSourceStats } from "./event-buffer";

/**
 * Wire-format launcher event. Matches the JSON serialization of
 * `agentmux_common::ipc::Event` (`#[serde(tag = "event", rename_all = "snake_case")]`).
 */
export type LauncherEvent = VersionedEvent;

const [latestEvent, setLatestEvent] = createSignal<LauncherEvent | null>(null);
const [eventVersion, setEventVersion] = createSignal<number>(0);
const [seenAnyEvent, setSeenAnyEvent] = createSignal<boolean>(false);

/** Latest typed event delivered by the launcher. Null until the first event. */
export const launcherEvent = latestEvent;

/** Monotonic version of the most recent event. 0 until the first event. */
export const launcherEventVersion = eventVersion;

/**
 * True once we've received at least one typed event. Kept as a
 * forward-compat utility — B.7.3.3 retired its only consumer (the
 * bespoke-channel gate in `app-init.ts`), but Phase D's `GetSnapshot`
 * resync flow may need it again.
 */
export const launcherEventsActive = seenAnyEvent;

const tracker = new PerSourceTracker<LauncherEvent>(
    { source: "launcher" },
    {
        setLatest: setLatestEvent,
        setVersion: setEventVersion,
        setSawAny: setSeenAnyEvent,
    },
);

/**
 * Per-event subscriber for the launcher pipe. Same contract as
 * `subscribeSrvEvent`: every event in source order, even during
 * saga buffer flushes. Returns an unsubscribe function.
 */
export function subscribeLauncherEvent(cb: EventCallback<LauncherEvent>): () => void {
    return tracker.subscribe(cb);
}

/** Diagnostic snapshot. Used by `--diag launcher` / tests. */
export function launcherEventStats(): PerSourceStats {
    return tracker.stats();
}

let installed = false;

// Drift-storm guard (v0.33.655 smoke): the host fanned a single
// `hwnd_drift_detected` event ~600× to the renderer in 3 seconds —
// V8 stack exhaustion → renderer crash. The launcher-side reset
// (`handle_report_pool_window_promoted` setting
// `foregrounded_since_open=true`) stops the storm at source; this
// per-key cache is the renderer-side defence-in-depth so a future
// upstream regression can't crash us. Keyed by
// `(event, label, hwnd)`; max version observed wins. PerSourceTracker
// already drops by global monotonic version, but a per-key check
// catches duplicates that the global counter advances past.
//
// See `docs/specs/ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`.
const MAX_DEDUP_KEYS = 1024;
const dedupSeen = new Map<string, number>();
let dedupSuppressedCount = 0;

function dedupKey(evt: LauncherEvent): string {
    const e = evt as LauncherEvent & { label?: unknown; hwnd?: unknown };
    const label = typeof e.label === "string" ? e.label : "";
    const hwnd = typeof e.hwnd === "number" ? e.hwnd : "";
    return `${evt.event}|${label}|${hwnd}`;
}

/**
 * Per-key (event-kind, label, hwnd) version-monotonicity guard.
 * Returns true if the event should be dispatched, false if it's a
 * duplicate or stale re-emission for the same logical entity.
 */
export function shouldDispatchLauncherEvent(evt: LauncherEvent): boolean {
    const k = dedupKey(evt);
    const seenVersion = dedupSeen.get(k);
    if (seenVersion !== undefined && evt.version <= seenVersion) {
        dedupSuppressedCount++;
        return false;
    }
    dedupSeen.set(k, evt.version);
    if (dedupSeen.size > MAX_DEDUP_KEYS) {
        // Bound memory. Map iteration order is insertion order → drop
        // the oldest entry. Acceptable: a re-arrival for an evicted
        // key can only re-fire if it carries a higher launcher
        // version than this renderer has ever seen, which means it's
        // not actually a duplicate.
        const firstKey = dedupSeen.keys().next().value;
        if (firstKey !== undefined) dedupSeen.delete(firstKey);
    }
    return true;
}

/** Diagnostic accessor for the dedup guard — used by tests + diag tools. */
export function launcherEventDedupStats(): { tracked: number; suppressed: number } {
    return { tracked: dedupSeen.size, suppressed: dedupSuppressedCount };
}

/** Test-only: clear the dedup cache. Not exported in production paths. */
export function __resetDedupForTests(): void {
    dedupSeen.clear();
    dedupSuppressedCount = 0;
}

/**
 * Register `window.__agentmux_launcher_event` as the dispatcher.
 * Idempotent — safe to call multiple times. Called once from
 * `app-init.ts::initApp` BEFORE the first state-needing operation
 * so events that arrive during init aren't dropped.
 */
export function installLauncherEventBridge(): void {
    if (installed) return;
    installed = true;
    (window as any).__agentmux_launcher_event = (evt: LauncherEvent) => {
        if (!shouldDispatchLauncherEvent(evt)) return;
        tracker.deliver(evt);
    };
    console.log("[launcher-events] bridge installed; window.__agentmux_launcher_event ready");
}
