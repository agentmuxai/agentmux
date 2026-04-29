// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.7.3.1 — renderer-side subscriber for launcher typed events.
//
// Mirrors `agentmux_common::ipc::Event` on the wire. The host's CEF
// JS bridge (`agentmux-cef/src/launcher_event_bridge.rs`) calls
// `window.__agentmux_launcher_event(<json>)` once per top-level
// renderer per launcher event. We feed those into a SolidJS signal
// so block-level subscribers can `createEffect()` on them.
//
// During the cutover (B.7.3.1), the bespoke `window-instances-changed`
// CEF event channel is still authoritative for the InstancePanel.
// This module just installs the cable. B.7.3.2 makes typed events
// the authoritative path for atom updates; B.7.3.3 retires the
// bespoke channel.
//
// `task dev` mode: the launcher isn't in the loop, so no events
// arrive. The dispatcher is still installed (idempotent, no
// side-effects), and downstream subscribers see no signal updates.
// The bespoke `window-instances-changed` channel continues to drive
// the UI in that mode until Phase E retires the no-launcher path.
//
// See `docs/specs/SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29.md`.

import { createSignal } from "solid-js";

/**
 * Wire-format launcher event. Matches the JSON serialization of
 * `agentmux_common::ipc::Event` (`#[serde(tag = "event", rename_all = "snake_case")]`).
 *
 * Every event carries `event` (discriminant, snake_case) and
 * `version` (monotonic per launcher run, used for de-dup / echo-loop
 * guard). Other fields are variant-specific; downstream subscribers
 * narrow on `event` and read fields by name.
 */
export interface LauncherEvent {
    event: string;
    version: number;
    [field: string]: unknown;
}

const [latestEvent, setLatestEvent] = createSignal<LauncherEvent | null>(null);
const [eventVersion, setEventVersion] = createSignal<number>(0);
const [seenAnyEvent, setSeenAnyEvent] = createSignal<boolean>(false);

/** Latest typed event delivered by the launcher. Null until the first event. */
export const launcherEvent = latestEvent;

/** Monotonic version of the most recent event. 0 until the first event. */
export const launcherEventVersion = eventVersion;

/**
 * True once we've received at least one typed event. The reducer uses
 * this as the signal that typed events are flowing — the renderer
 * can treat them as authoritative once this flips. Until then, the
 * bespoke `window-instances-changed` channel remains the source of
 * truth (covers `task dev` mode where no launcher is in the loop).
 */
export const launcherEventsActive = seenAnyEvent;

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
    (window as any).__agentmux_launcher_event = (evt: LauncherEvent) => {
        if (!evt || typeof evt.version !== "number" || typeof evt.event !== "string") {
            console.warn("[launcher-events] received malformed event", evt);
            return;
        }
        setLatestEvent(evt);
        setEventVersion(evt.version);
        if (!seenAnyEvent()) {
            setSeenAnyEvent(true);
        }
    };
    console.log("[launcher-events] bridge installed; window.__agentmux_launcher_event ready");
}
