// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The newly-shown guard (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md G6): a pane or panel that has just
 * appeared ignores clicks and Enter for a short fixed time, so a press meant for what was under the pointer or the
 * caret does not land on it. One constant, read by every guard that needs it (the deep-link executor and the Vault
 * request card's buttons, task #48141), so the two cannot drift into two numbers.
 */

/** How long a newly shown pane or panel ignores clicks and Enter. */
export const NEWLY_SHOWN_GUARD_MS = 800;

/** Whether a press at `now` is inside the guard of something shown at `shownAt` (both in `performance.now()` ms). */
export function withinNewlyShownGuard(shownAt: number, now: number = performance.now()): boolean {
    return now >= shownAt && now - shownAt < NEWLY_SHOWN_GUARD_MS;
}

/**
 * Makes `el` ignore clicks and Enter for the guard time from now: they are stopped before anything inside sees them.
 * Returns a function that ends the guard early (and is called by itself when the time is up).
 */
export function guardNewlyShown(el: HTMLElement, now: () => number = () => performance.now()): () => void {
    const shownAt = now();
    const swallow = (ev: Event) => {
        if (ev instanceof KeyboardEvent && ev.key !== "Enter") return;
        if (!withinNewlyShownGuard(shownAt, now())) return;
        ev.preventDefault();
        ev.stopPropagation();
    };
    const kinds = ["click", "pointerdown", "mousedown", "keydown"] as const;
    for (const k of kinds) el.addEventListener(k, swallow, true);
    let ended = false;
    const end = () => {
        if (ended) return;
        ended = true;
        clearTimeout(timer);
        for (const k of kinds) el.removeEventListener(k, swallow, true);
    };
    const timer = setTimeout(end, NEWLY_SHOWN_GUARD_MS);
    return end;
}
