// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Window-opacity store — dispatch layer for the window-opacity reducer slice.
 * See SPEC_PER_WINDOW_OPACITY_2026-05-14.md §7.2.
 *
 * Responsibilities:
 *   - Hold the reducer's current state.
 *   - Route WindowOpacityCommand through `update()`.
 *   - Apply IPC side-effects (setWindowOpacity) for emitted events.
 *     The reducer itself performs no I/O.
 */

import { getApi } from "@/store/global";
import { update } from "./window-opacity/reducer";
import { initialState, type WindowOpacityCommand, type WindowOpacityState } from "./window-opacity/types";

let currentState: WindowOpacityState = initialState();

export function dispatchWindowOpacity(command: WindowOpacityCommand): void {
    const { state: nextState, events } = update(currentState, command);
    currentState = nextState;

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

/** Read-only snapshot of the current opacity for a window. */
export function getWindowOpacityState(windowId: string): number {
    return currentState.opacities[windowId] ?? 1.0;
}
