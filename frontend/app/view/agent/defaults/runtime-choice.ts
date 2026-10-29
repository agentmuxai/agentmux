// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The runtime (Container or Native) the person last chose themselves in the
 * new-agent form, kept on this machine like the agent list's sort order. The
 * form preselects it next time (`shouldPreselectContainer`); a default the
 * probes picked is never stored. Best effort: storage can throw in some
 * embedding contexts, and a lost preference is not worth a failure.
 */
const LAST_RUNTIME_KEY = "agentCreate:lastRuntime";

export type RuntimeChoice = "host" | "container";

export function loadLastRuntime(): RuntimeChoice | null {
    try {
        const raw = localStorage.getItem(LAST_RUNTIME_KEY);
        if (raw === "host" || raw === "container") return raw;
    } catch {
        // fall through: no remembered choice
    }
    return null;
}

export function storeLastRuntime(choice: RuntimeChoice): void {
    try {
        localStorage.setItem(LAST_RUNTIME_KEY, choice);
    } catch {
        // best effort only
    }
}
