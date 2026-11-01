// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The sort the person last chose in the agent picker, kept on this machine. Split out of AgentPicker.tsx (#13044 S6).

import { DEFAULT_AGENT_SORT, type AgentSortOption } from "./AgentPickerFilterBar";

/** This-machine-only preference — no cross-device sync, no existing
 *  localStorage precedent in this component tree to extend (a new small
 *  pattern, not a reuse). Wrapped defensively: localStorage can throw in
 *  some embedding contexts (private browsing, storage quota), and a sort
 *  preference is never worth a hard failure over. */
const AGENT_SORT_STORAGE_KEY = "agentPicker:sortBy";

export function loadStoredSort(): AgentSortOption {
    try {
        const raw = localStorage.getItem(AGENT_SORT_STORAGE_KEY);
        if (raw === "recent" || raw === "name" || raw === "type") return raw;
    } catch {
        // ignore — fall through to the default
    }
    return DEFAULT_AGENT_SORT;
}

export function storeSort(sort: AgentSortOption): void {
    try {
        localStorage.setItem(AGENT_SORT_STORAGE_KEY, sort);
    } catch {
        // best-effort only — a failed write just means the preference
        // doesn't survive to the next picker open, not a functional break
    }
}
