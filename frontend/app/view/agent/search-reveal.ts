// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Search over a pane whose view hides rows (SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29 §4.6): the search reads
 * every row, so a match can sit inside a hidden run. Jumping to it opens that run in place (the run's pin, as
 * a click on its line does). Only the run that holds the current match stays open (SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE
 * §4.3): the step moving on, the query ending up with no match, and the search closing each close the runs it
 * opened that are not needed now. A run the person had opened themselves stays open.
 */

import type { DocumentNode, HiddenRunNode } from "../../agent-model/types";

export interface SearchRevealOptions {
    /** The rows the pane shows now, hidden-run lines included. */
    shown: () => DocumentNode[];
    /** Change the pane's pinned set (a hidden run is open while its id is in it). */
    setPins: (update: (pins: Set<string>) => Set<string>) => void;
    /** The pane's own jump to a row. */
    jumpTo: (nodeId: string) => void;
    /** Run a jump once the opened run's rows are laid out; the default waits two frames. */
    afterOpen?: (jump: () => void) => void;
}

const afterTwoFrames = (jump: () => void) => requestAnimationFrame(() => requestAnimationFrame(jump));

export function createSearchReveal(opts: SearchRevealOptions) {
    const opened = new Set<string>();
    // Bumped by every jump and by the search ending: a jump waiting for layout that finds it changed is stale
    // (a newer match was asked for, or the run is closed again) and does nothing.
    let latest = 0;
    /** Close the runs the search opened, except `keep` (the run the current match is in). */
    const closeOpened = (keep: string | null): void => {
        const ids = [...opened].filter((id) => id !== keep);
        if (ids.length === 0) return;
        for (const id of ids) opened.delete(id);
        opts.setPins((pins) => {
            const next = new Set(pins);
            for (const id of ids) next.delete(id);
            return next;
        });
    };
    return {
        jumpTo(nodeId: string): void {
            const mine = ++latest;
            const holding = opts
                .shown()
                .find((n): n is HiddenRunNode => n.type === "hidden_run" && n.hiddenIds.includes(nodeId));
            closeOpened(holding && opened.has(holding.id) ? holding.id : null);
            const run = holding && !holding.open ? holding : undefined;
            if (!run) return opts.jumpTo(nodeId);
            // The row has no place in the list until its run is open and laid out: a jump now would find none.
            opened.add(run.id);
            opts.setPins((pins) => new Set(pins).add(run.id));
            (opts.afterOpen ?? afterTwoFrames)(() => {
                if (mine === latest) opts.jumpTo(nodeId);
            });
        },
        /** The search ended, or no match is current any more: close the runs it opened. */
        release(): void {
            latest++;
            closeOpened(null);
        },
    };
}
