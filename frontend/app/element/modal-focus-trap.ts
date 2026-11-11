// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// ── Helpers ──────────────────────────────────────────────────────────────────

const FOCUSABLE_SELECTOR = [
    "input:not([disabled])",
    "textarea:not([disabled])",
    "select:not([disabled])",
    "button:not([disabled])",
    "a[href]",
    "[tabindex]:not([tabindex='-1'])",
].join(",");

export function firstFocusable(root: HTMLElement): HTMLElement | null {
    return root.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
}

export function lastFocusable(root: HTMLElement): HTMLElement | null {
    const nodes = root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR);
    return nodes.length ? nodes[nodes.length - 1] : null;
}

/**
 * Where focus lands when a modal opens or its content is replaced: the
 * element the panel marked with `data-modal-initial-focus` (its primary
 * action), else the first focusable. Without the marker, a panel whose
 * first focusable is a log or terminal would open scrolled to it.
 */
export function initialFocusTarget(root: HTMLElement): HTMLElement | null {
    return root.querySelector<HTMLElement>("[data-modal-initial-focus]:not([disabled])") ?? firstFocusable(root);
}

/**
 * The pane a modal over `region` must wait for before it takes the caret: the pane it covers, when the person
 * has another pane selected (`selected`). A question that comes up in a pane they are not in (srv's, an
 * agent's, a page's) must not take the keyboard from the terminal or composer they are typing in: a Space
 * or Enter meant for it would answer the question. None when the modal is in the selected pane, when no pane
 * is selected, or when `region` is in no pane (a tab's or the window's modal).
 */
export function paneToWaitFor(region: HTMLElement, selected: () => string | null): string | null {
    const blockId = region.closest<HTMLElement>("[data-blockid]")?.dataset.blockid;
    if (!blockId) return null;
    const now = selected();
    return now != null && now !== blockId ? blockId : null;
}
