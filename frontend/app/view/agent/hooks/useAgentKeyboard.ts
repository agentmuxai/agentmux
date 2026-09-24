// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentKeyboard — pane-scoped Ctrl+F listener.
 *
 * Step 10 of docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Installs a window-level keydown handler on mount and removes it
 * on cleanup. The handler early-exits unless the key belongs to THIS pane
 * (`eventBelongsToBlock`: the pane the key came from, else the selected pane),
 * so only one agent pane responds when several are open. It used
 * `focusedBlockId()`, whose text-selection fallback sent a Ctrl+F typed in
 * pane B to pane A whenever text was left selected in A.
 *
 * Ctrl+F — toggle the search bar. Second press closes it (caller's
 *          `onToggleSearch` is responsible for clearing state).
 */

import { onCleanup, onMount } from "solid-js";
import { eventBelongsToBlock } from "@/util/focusutil";

export interface UseAgentKeyboardOptions {
    blockId: string;
    /** Called on Ctrl+F when this pane is focused. */
    onToggleSearch: () => void;
}

export function useAgentKeyboard(opts: UseAgentKeyboardOptions): void {
    onMount(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (!eventBelongsToBlock(e, opts.blockId)) return;

            if (e.ctrlKey && e.key === "f") {
                e.preventDefault();
                opts.onToggleSearch();
            }
        };
        window.addEventListener("keydown", handleKeyDown);
        onCleanup(() => window.removeEventListener("keydown", handleKeyDown));
    });
}
