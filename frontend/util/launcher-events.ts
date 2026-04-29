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
// Phase B.7.3.3 — typed events are the SOLE path for InstancePanel
// state. The bespoke `window-instances-changed` channel and its 4
// sync emit sites in the host are gone. `task dev` mode (no
// launcher in the loop) currently leaves the InstancePanel atoms
// at their seeded values from the init RPC; live updates require
// the launcher. Phase E folds srv into the same reducer pattern;
// the no-launcher mode goes away then.
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
 * True once we've received at least one typed event. Kept as a
 * forward-compat utility — B.7.3.3 retired its only consumer (the
 * bespoke-channel gate in `app-init.ts`), but Phase D's `GetSnapshot`
 * resync flow may need it again to detect "have we resynced?" vs
 * "fresh launcher / first events not seen yet."
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
