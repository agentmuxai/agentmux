// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, type Accessor } from "solid-js";

/**
 * Process-wide signal indicating whether the current renderer is hosting a
 * floating-pane shell and, if so, the floater's window label + maximize
 * state.
 *
 * Set by `FloatingPaneWorkspace.onMount` (`frontend/app/workspace/
 * floating-pane-workspace.tsx`); cleared by its `onCleanup`. Read by
 * `EndIcons` in `frontend/app/block/blockframe.tsx` so the maximize button
 * only renders in the floating shell — docked panes never see it.
 *
 * `state` mirrors the host's `GetWindowPlacement().showCmd` and is updated
 * from the return value of `maximize_window`. We avoid emitting a
 * `window_state_changed` event from the host wndproc in this phase — the
 * IPC's return value covers the user-initiated cases (header button,
 * dblclick); follow-up work can wire the event channel for keyboard/system
 * maximize paths.
 *
 * See SPEC_FLOATING_PANE_RESIZE_AND_MAXIMIZE_2026-05-28.md §5.4.
 */
export interface FloatingPaneInfo {
    windowLabel: string;
    state: "normal" | "maximized";
}

const [floatingPaneInfo, setFloatingPaneInfo] =
    createSignal<FloatingPaneInfo | null>(null);

export const useFloatingPaneInfo: Accessor<FloatingPaneInfo | null> =
    floatingPaneInfo;

export { setFloatingPaneInfo };
