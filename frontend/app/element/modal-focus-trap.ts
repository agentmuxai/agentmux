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
