// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ModalScope } from "./modal";

// ── Modal stack (SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21 §6) ──────────────────
// Module-level so every Modal instance shares it. Each entry records the
// modal's `scope` and `lockEl` so ESC / backdrop dispatch can reason about
// scope containment (e.g. a pane modal stacked inside a window modal stays
// dismissable on its own ESC; window modal stays untouched).
//
// The "reachable topmost" is the highest-stacked modal NOT contained
// within a higher modal's lock region. Modals whose lock regions don't
// overlap (two pane modals in different panes; a pane modal + a tab
// modal in another tab) coexist — each stays independently reachable.

export interface StackEntry {
    id: string;
    scope: ModalScope;
    /** The element this modal locks (its backdrop / inert region). */
    lockEl: HTMLElement;
    close: () => void;
}

const stack: StackEntry[] = [];

export const push = (entry: StackEntry): void => {
    stack.push(entry);
};

export const remove = (id: string): void => {
    const idx = stack.findIndex((e) => e.id === id);
    if (idx >= 0) stack.splice(idx, 1);
};

/**
 * True when `inner`'s lock region is covered by `outer`'s — i.e. `outer`
 * is a higher modal whose backdrop blocks interaction with `inner`. A
 * region is covered if `outer.lockEl` contains `inner.lockEl`, or the two
 * resolve to the same node (two window modals share `document.body`).
 */
function covers(outer: StackEntry, inner: StackEntry): boolean {
    if (outer.lockEl === inner.lockEl) return true;
    return outer.lockEl.contains(inner.lockEl);
}

/**
 * True when `self` should respond to a global ESC / backdrop interaction:
 * no modal stacked *above* `self` covers its lock region.
 *
 * This is per-region, not a single global winner — modals in disjoint
 * regions (two pane modals in different panes; a pane modal and a tab
 * modal in another tab) are each independently reachable. Only a higher
 * modal whose lock region *contains* `self`'s shadows it.
 */
export function isReachable(self: StackEntry): boolean {
    const i = stack.findIndex((e) => e.id === self.id);
    if (i < 0) return false;
    for (let j = i + 1; j < stack.length; j++) {
        if (covers(stack[j], self)) return false;
    }
    return true;
}
