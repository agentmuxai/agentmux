// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Module-level registry for per-pane shell history (sent messages).
 *
 * Follows the same pattern as `token-usage.ts`: a `Map<blockId, …>`
 * holds a reactive signal per pane so `AgentShellHistoryPanel` can
 * read and `AgentFooter` can push without lifting state into the
 * reducer. Shell history has no cross-field invariants against
 * `turnPhase` / `detailsOpen` / etc., so it doesn't belong in
 * `AgentPaneState` ("no god-reducer" §11).
 *
 * Spec: docs/specs/SPEC_AGENT_COMPOSER_STRIP_REDESIGN_2026_06_23.md §5.
 */

import { createSignal, type Accessor } from "solid-js";

const MAX_HISTORY = 50;

interface ShellHistoryEntry {
    get: Accessor<string[]>;
    push: (msg: string) => void;
}

const registry = new Map<string, ShellHistoryEntry>();

export function getShellHistory(blockId: string): ShellHistoryEntry {
    let entry = registry.get(blockId);
    if (!entry) {
        const [get, set] = createSignal<string[]>([]);
        entry = {
            get,
            push: (msg: string) =>
                set((prev) => {
                    if (prev[0] === msg) return prev; // skip consecutive duplicate
                    const next = [msg, ...prev];
                    return next.length > MAX_HISTORY ? next.slice(0, MAX_HISTORY) : next;
                }),
        };
        registry.set(blockId, entry);
    }
    return entry;
}

export function clearShellHistory(blockId: string): void {
    registry.delete(blockId);
}
