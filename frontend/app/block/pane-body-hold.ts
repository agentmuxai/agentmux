// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's body holds the size it had when a splitter drag began, and the pane's frame, header and the border
 * follow the pointer. While a border is dragged, each pointer move used to lay out every pane at its new size: a
 * terminal refitted its grid, an agent pane reflowed its rows, an editor relaid out, all of it on every frame, and
 * the drag felt slow. Held, the body keeps its size and is clipped by the frame (the frame's own background shows
 * in what a growing pane gains), so none of that happens during the drag; on release the hold is dropped and each
 * body lays out once at the final size, which is when a terminal refits and the pty hears of it.
 *
 * Which panes: every pane of the tab whose layout is being dragged (a pane that does not change size holds the
 * size it already has). Not a browser pane: its page is a native window that follows the pane's rect, so a held
 * body would leave it at the old size over its neighbour (it settles as it did).
 */

import { createEffect, createSignal, untrack, type Accessor } from "solid-js";

/** The size, in CSS px, of the box a pane's view is laid out in: its content box. */
export interface BodyHold {
    width: number;
    height: number;
}

/** Views whose body is never held: a native window follows the pane's rect. */
const NEVER_HELD_VIEWS: ReadonlySet<string> = new Set(["browser"]);

/** Whether a pane showing `view` holds its body during a splitter drag. */
export function holdsBodyDuringDrag(view: string | undefined): boolean {
    return !(view !== undefined && NEVER_HELD_VIEWS.has(view));
}

/**
 * The content box of `content` (`.block-content`), the size its view has now: its inner size less its padding.
 * Null when it has none (not laid out, hidden).
 */
export function measureBodyHold(content: HTMLElement): BodyHold | null {
    const style = getComputedStyle(content);
    const px = (v: string) => Number.parseFloat(v) || 0;
    const width = content.clientWidth - px(style.paddingLeft) - px(style.paddingRight);
    const height = content.clientHeight - px(style.paddingTop) - px(style.paddingBottom);
    return width > 0 && height > 0 ? { width, height } : null;
}

/**
 * The size this pane's body holds, or null: measured once when a splitter drag begins, kept for the whole drag
 * however the pane is resized under it, and dropped when the drag ends. Declare it before any memo that reads it.
 */
export function usePaneBodyHold(
    dragging: Accessor<boolean>,
    content: () => HTMLElement | null | undefined,
    view: Accessor<string | undefined>,
): Accessor<BodyHold | null> {
    const [hold, setHold] = createSignal<BodyHold | null>(null);
    createEffect(() => {
        if (!dragging()) return setHold(null);
        const el = content();
        if (el && holdsBodyDuringDrag(untrack(view))) setHold(measureBodyHold(el));
    });
    return hold;
}

/** The custom properties `.block-content--held` (block.scss) reads. */
export function bodyHoldVars(hold: BodyHold | null): Record<string, string> {
    return hold ? { "--held-width": `${hold.width}px`, "--held-height": `${hold.height}px` } : {};
}
