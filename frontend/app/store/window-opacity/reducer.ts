// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pure reducer for the window-opacity slice.
 * See SPEC_PER_WINDOW_OPACITY_2026-05-14.md §7.2 and
 * docs/specs/frontend-reducer-conventions-2026-05-03.md.
 *
 * Invariants:
 *   1. Opacity is clamped to [0.35, 1.0] at the reducer boundary.
 *      Values ≥ 1.0 remove the entry (fully opaque = absent).
 *   2. No I/O inside the reducer. IPC calls live in window-opacity-store.ts.
 *   3. WindowClosed removes the entry without emitting an IPC event —
 *      the native window is already gone by the time this fires.
 *
 * Keyed by window label — covers both main windows and floating panes
 * (instance-panel-floating-panes.md §3.2).
 */

import type {
    ReducerResult,
    WindowOpacityCommand,
    WindowOpacityState,
} from "./types";

const OPACITY_MIN = 0.35;

export function update(
    state: WindowOpacityState,
    command: WindowOpacityCommand,
): ReducerResult {
    switch (command.type) {
        case "SetWindowOpacity": {
            const opacity = Math.max(OPACITY_MIN, Math.min(1.0, command.opacity));
            if (opacity >= 1.0) {
                const { [command.label]: _, ...rest } = state.opacities;
                return {
                    state: { opacities: rest },
                    events: [
                        {
                            type: "window-opacity-cleared",
                            label: command.label,
                        },
                    ],
                };
            }
            return {
                state: {
                    opacities: { ...state.opacities, [command.label]: opacity },
                },
                events: [
                    {
                        type: "window-opacity-applied",
                        label: command.label,
                        opacity,
                    },
                ],
            };
        }

        case "WindowClosed": {
            const { [command.label]: _, ...rest } = state.opacities;
            return {
                state: { opacities: rest },
                events: [{ type: "window-opacity-entry-removed", label: command.label }],
            };
        }

        default:
            return { state, events: [] };
    }
}
