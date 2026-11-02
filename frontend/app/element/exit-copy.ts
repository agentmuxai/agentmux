// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// For an element whose owner removes it from the page at once (a `<Show>`, a `<For>` row): a
// non-interactive copy of it stays where it was and plays `exit`, then removes itself. The owner's
// logic is untouched, so nothing else has to wait for the motion.

import { exit, MOTION, motionAllowed, type EnterKind } from "@/app/motion/primitives";

/** Make `copy` a picture of `el`: nothing in it can be pressed, focused, found by a test or announced. */
function quiet(copy: HTMLElement): void {
    copy.setAttribute("aria-hidden", "true");
    copy.setAttribute("inert", "");
    copy.style.pointerEvents = "none";
    for (const node of [copy, ...copy.querySelectorAll<HTMLElement>("[id], [data-testid], [data-pane-overlay]")]) {
        node.removeAttribute("id");
        node.removeAttribute("data-testid");
        node.removeAttribute("data-pane-overlay");
    }
}

/**
 * Fade out a copy of `el`, for a Solid cleanup (which runs before the node leaves the page).
 * `fixed` is for an `el` that is not itself `position: fixed`: the copy is pinned at its rectangle.
 * With motion off nothing is copied.
 */
export function fadeOutCopy(el: Element, kind: EnterKind = "fade", opts: { fixed?: boolean } = {}): void {
    if (typeof document === "undefined") return;
    if (!motionAllowed(MOTION.exit.category)) return;
    const copy = el.cloneNode(true) as HTMLElement;
    quiet(copy);
    if (opts.fixed) {
        if (!el.isConnected) return;
        const rect = el.getBoundingClientRect();
        Object.assign(copy.style, {
            position: "fixed",
            left: `${rect.left}px`,
            top: `${rect.top}px`,
            width: `${rect.width}px`,
            height: `${rect.height}px`,
            margin: "0",
        });
    }
    document.body.append(copy);
    void exit(copy, kind).finished.then(() => copy.remove());
}
