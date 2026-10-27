// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Open the Supervisor's settings panel": asked from the Settings pane's search, answered by the
// panel in the Swarm's Supervisor section, which may not be mounted yet when the ask is made. Kept
// in the store layer so neither view imports the other.
// docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §6.2.

import { createSignal } from "solid-js";

let pending = false;
const [requests, setRequests] = createSignal(0);

/** Count of asks so far; a panel that is already mounted watches it. */
export const supervisorSettingsRequests = requests;

/** Ask for the panel to open: now if a Swarm shows it, else when one mounts. */
export function requestSupervisorSettingsOpen(): void {
    pending = true;
    setRequests((n) => n + 1);
}

/** The panel's side: whether an ask is waiting, taking it (so one ask opens one panel). */
export function takeSupervisorSettingsRequest(): boolean {
    const was = pending;
    pending = false;
    return was;
}

/** For tests. */
export function resetSupervisorSettingsRequest(): void {
    pending = false;
}
