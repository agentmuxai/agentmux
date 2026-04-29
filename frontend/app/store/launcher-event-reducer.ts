// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.7.3.1 — frontend reducer for launcher typed events.
//
// Subscribes to the `launcherEvent` signal from
// `frontend/util/launcher-events.ts` and dispatches by `event`
// discriminant. In B.7.3.1 this is SCAFFOLDING: it logs every typed
// event and, for the variants that already have a corresponding
// bespoke `window-instances-changed` payload path, runs alongside
// the bespoke channel (idempotent — both write the same atoms).
//
// B.7.3.2 promotes typed events to the authoritative path: when
// `launcherEventsActive()` is true, atom-update side-effects flow
// from here, not from the bespoke listener.
// B.7.3.3 retires the bespoke channel and its 4 sync emit sites.
//
// Echo-loop guard (parent spec §5.5): an `applyingRemote` flag is
// set during each apply so future renderer-emitted commands can
// detect "this state change came from the launcher; don't re-emit."
// Currently no commands flow through this bridge (commands still
// take the host IPC HTTP path), so the flag is forward-compatibility
// scaffolding.
//
// See `docs/specs/SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29.md`.

import { createEffect } from "solid-js";

import { launcherEvent, launcherEventVersion, type LauncherEvent } from "@/util/launcher-events";

let applyingRemote = false;

/**
 * True while the reducer is mid-apply for a launcher event. Future
 * renderer-emitted commands should check this and skip re-emission
 * to avoid echo loops with the launcher.
 */
export function isApplyingRemoteEvent(): boolean {
    return applyingRemote;
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
            dispatch(evt);
        } finally {
            applyingRemote = false;
        }
    });
}

function dispatch(evt: LauncherEvent): void {
    // B.7.3.1: log every event so smoke tests can verify the cable.
    // B.7.3.2 promotes specific arms to atom-mutating handlers.
    console.log("[launcher-event-reducer]", evt.event, "v=" + evt.version, evt);
    switch (evt.event) {
        case "window_opened":
        case "window_closed":
        case "window_instance_assigned":
        case "window_instance_released":
        case "backend_window_id_registered":
        case "backend_window_id_unregistered":
            // Lifecycle events — currently fed via the bespoke
            // `window-instances-changed` payload's resolved entries.
            // B.7.3.2 will swap that for an in-memory reduction
            // here and demote the bespoke channel.
            break;
        case "hwnd_drift_detected":
            // Drift events: WRR emits these when the launcher's
            // pure-reducer detects an off-monitor / hidden / orphan
            // window. Currently logged only; UX wiring (toast or
            // debug panel) is a separate question (spec §Open
            // questions #3).
            break;
        case "corrective_window_move":
        case "host_should_quit":
            // Saga events handled host-side via launcher_ipc.rs.
            // Renderer just logs; no UI action.
            break;
        default:
            // Forward-compat: unknown variants are silently ignored.
            // The host bridge forwards every Event variant, so newly-
            // added events (e.g. Phase D snapshot deltas) flow here
            // without renderer code change until they need handling.
            break;
    }
}
