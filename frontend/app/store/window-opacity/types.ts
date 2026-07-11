// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Type definitions for the window-opacity reducer slice.
 * See SPEC_PER_WINDOW_OPACITY_2026-05-14.md §7.2 and
 * docs/specs/frontend-reducer-conventions-2026-05-03.md.
 *
 * Tracks the current opacity (0.35–1.0) for each window by **label**.
 * Keyed by label (not windowId) since instance-panel-floating-panes.md §3.2:
 * floating panes participate in the slider UI but have `windowId: null` —
 * labels are unique across both main windows and floaters, and the IPC
 * side-effect (`setWindowOpacity(label, …)`) is label-addressed anyway.
 * Absent = fully opaque (1.0). Win32 side-effect (SetLayeredWindowAttributes)
 * is applied by the dispatch layer, not inside the reducer.
 */

/** Reducer state — per-window opacity map. */
export interface WindowOpacityState {
    /** window label → opacity in [0.35, 1.0]. Absent means fully opaque. */
    opacities: Record<string, number>;
}

export const initialState = (): WindowOpacityState => ({
    opacities: {},
});

export type WindowOpacityCommand =
    /**
     * User dragged the opacity slider or restored a saved opacity.
     * `source: "user"` — immediate IPC call in dispatch layer.
     * `source: "restore"` — called from app-init.ts at window load.
     */
    | {
          type: "SetWindowOpacity";
          label: string;
          opacity: number;
          source: "user" | "restore";
      }
    /** Window (or floating pane) was closed — clean up its opacity entry. */
    | { type: "WindowClosed"; label: string };

export type WindowOpacityEvent =
    /** Opacity applied (0.35 ≤ opacity < 1.0). Dispatch layer fires IPC. */
    | { type: "window-opacity-applied"; label: string; opacity: number }
    /** Opacity cleared (≥ 1.0 → remove WS_EX_LAYERED). Dispatch layer fires IPC. */
    | { type: "window-opacity-cleared"; label: string }
    /** Window closed — entry removed, no IPC call needed. */
    | { type: "window-opacity-entry-removed"; label: string };

export interface ReducerResult {
    state: WindowOpacityState;
    events: WindowOpacityEvent[];
}
