// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Search over a pane whose view hides rows (SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29 §4.6): the search reads
 * every row, so a match can sit inside a hidden run. Jumping to it opens that run in place (the run's pin, as
 * a click on its line does) and the search closing closes the runs it opened again. A run the person had
 * opened themselves stays open.
 */

import type { DocumentNode } from "../../agent-model/types";

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
    return {
        jumpTo(nodeId: string): void {
            const run = opts.shown().find((n) => n.type === "hidden_run" && !n.open && n.hiddenIds.includes(nodeId));
            if (!run) return opts.jumpTo(nodeId);
            // The row has no place in the list until its run is open and laid out: a jump now would find none.
            opened.add(run.id);
            opts.setPins((pins) => new Set(pins).add(run.id));
            (opts.afterOpen ?? afterTwoFrames)(() => opts.jumpTo(nodeId));
        },
        /** The search ended: close the runs it opened. */
        release(): void {
            if (opened.size === 0) return;
            const ids = [...opened];
            opened.clear();
            opts.setPins((pins) => {
                const next = new Set(pins);
                for (const id of ids) next.delete(id);
                return next;
            });
        },
    };
}
