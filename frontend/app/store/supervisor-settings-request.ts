// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Open the Supervisor's settings panel": asked from the Settings pane's search, answered by the
// panel in the Swarm's Supervisor section, which may not be mounted yet when the ask is made. Kept
// in the store layer so neither view imports the other.
// docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §6.2.

import { createSignal } from "solid-js";

let pending = new Set<string>();
const [requests, setRequests] = createSignal(0);

/** Count of asks so far; a panel that is already mounted watches it. */
export const supervisorSettingsRequests = requests;

/** Ask the panel of one Swarm pane (`blockId`) to open: now if it shows, else when it mounts. A Swarm
 *  in another tab stays mounted and watches the same count, so the ask names the pane. */
export function requestSupervisorSettingsOpen(blockId: string): void {
    pending.add(blockId);
    setRequests((n) => n + 1);
}

/** The panel's side: whether an ask for this pane is waiting, taking it (so one ask opens one panel). */
export function takeSupervisorSettingsRequest(blockId: string | null | undefined): boolean {
    return blockId != null && pending.delete(blockId);
}

/** For tests. */
export function resetSupervisorSettingsRequest(): void {
    pending = new Set();
}
