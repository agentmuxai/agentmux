// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Window-opacity store — dispatch layer for the window-opacity reducer slice.
 * See SPEC_PER_WINDOW_OPACITY_2026-05-14.md §7.2.
 *
 * Responsibilities:
 *   - Hold the reducer's current state (a Solid signal, so UI can react to it).
 *   - Route WindowOpacityCommand through `update()`.
 *   - Apply IPC side-effects (setWindowOpacity) for emitted events.
 *     The reducer itself performs no I/O.
 */

import { createSignal } from "solid-js";

import { getApi } from "@/store/global";
import { update } from "./window-opacity/reducer";
import { initialState, type WindowOpacityCommand, type WindowOpacityState } from "./window-opacity/types";

// Module-level reactive state. A signal (not a plain `let`) so a UI reader —
// e.g. the status-bar opacity slider's % label — re-runs whenever opacity is
// dispatched, including on every tick of a slider drag. `dispatchWindowOpacity`
// is the only writer.
const [state, setState] = createSignal<WindowOpacityState>(initialState());

export function dispatchWindowOpacity(command: WindowOpacityCommand): void {
    const { state: nextState, events } = update(state(), command);
    setState(nextState);

    for (const event of events) {
        switch (event.type) {
            case "window-opacity-applied":
                getApi()
                    .setWindowOpacity(event.label, event.opacity)
                    .catch((e: unknown) =>
                        console.error("[window-opacity] setWindowOpacity IPC failed:", e),
                    );
                break;
            case "window-opacity-cleared":
                getApi()
                    .setWindowOpacity(event.label, 1.0)
                    .catch((e: unknown) =>
                        console.error("[window-opacity] clearWindowOpacity IPC failed:", e),
                    );
                break;
            case "window-opacity-entry-removed":
                // Window closed — no IPC needed.
                break;
        }
    }
}

/**
 * Reactive read of the live opacity the store currently holds for a window
 * or floating pane (keyed by label — instance-panel-floating-panes.md §3.2),
 * or `undefined` when the store has no entry yet (nothing dispatched for it
 * this session). Read inside a reactive context (e.g. JSX), it re-runs on
 * every `dispatchWindowOpacity` — so a slider's % label tracks the drag.
 */
export function liveWindowOpacity(label: string | undefined): number | undefined {
    if (!label) return undefined;
    return state().opacities[label];
}
