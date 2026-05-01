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

/**
 * Register `window.__agentmux_launcher_event` as the dispatcher.
 * Idempotent — safe to call multiple times. Called once from
 * `app-init.ts::initApp` BEFORE the first state-needing operation
 * so events that arrive during init aren't dropped.
 */
export function installLauncherEventBridge(): void {
    if (installed) return;
    installed = true;
    (window as any).__agentmux_launcher_event = (evt: LauncherEvent) => tracker.deliver(evt);
    console.log("[launcher-events] bridge installed; window.__agentmux_launcher_event ready");
}
