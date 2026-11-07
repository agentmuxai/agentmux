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
 * Ctrl+F — open the search bar, or put the caret in its field when it is open (the caller's `onToggleSearch`).
 * F3     — next match while the bar is open; Shift+F3 the previous one.
 * Alt+V  — cycle the pane's view (Full, Standard, Replies); Alt+Shift+V clears the pane's own view.
 *          Matched by key code: on macOS Option+V types a character.
 */

import { onCleanup, onMount } from "solid-js";
import { eventBelongsToBlock } from "@/util/focusutil";
import { isModKey } from "@/util/platformutil";

export interface UseAgentKeyboardOptions {
    blockId: string;
    /** Called on Ctrl+F when this pane is focused. */
    onToggleSearch: () => void;
    /** Whether the search bar is open: F3 steps only then. */
    searchOpen?: () => boolean;
    /** F3 (forward) and Shift+F3 (backward) with the bar open. */
    onStepSearch?: (backward: boolean) => void;
    /** Alt+V. */
    onCycleView?: () => void;
    /** Alt+Shift+V. */
    onFollowDefaultView?: () => void;
}

export function useAgentKeyboard(opts: UseAgentKeyboardOptions): void {
    onMount(() => {
        const handleKeyDown = (e: KeyboardEvent) => {
            if (!eventBelongsToBlock(e, opts.blockId)) return;

            if (isModKey(e) && e.key === "f") {
                e.preventDefault();
                opts.onToggleSearch();
                return;
            }
            if (e.key === "F3" && !e.altKey && !e.ctrlKey && !e.metaKey && opts.searchOpen?.()) {
                e.preventDefault();
                opts.onStepSearch?.(e.shiftKey);
                return;
            }
            if (e.altKey && !e.ctrlKey && !e.metaKey && e.code === "KeyV") {
                e.preventDefault();
                (e.shiftKey ? opts.onFollowDefaultView : opts.onCycleView)?.();
            }
        };
        window.addEventListener("keydown", handleKeyDown);
        onCleanup(() => window.removeEventListener("keydown", handleKeyDown));
    });
}
