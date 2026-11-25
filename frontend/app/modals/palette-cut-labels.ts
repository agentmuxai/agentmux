// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A label the palette cuts off is the one whose full text the row's tooltip gives (R14); a whole one needs no tooltip.
// Which labels are cut is known only from the layout, so it is read here for the whole list at once, in one pass that
// only reads (one layout) followed by one that only writes, after the frame that drew the rows: never row by row while
// they are built, which made every keystroke measure, write and measure again. After the frame is drawn, not in the
// rendering step before it: that read forced the layout of the rows into the first frame.

import { afterNextPaint } from "./palette-after-paint";

const ITEM = ".command-palette-item";
const LABEL = ".command-palette-item-label";

/** Gives each row whose label is cut its label as a tooltip, and takes it off a row whose label is whole. */
export function markCutLabels(list: HTMLElement): void {
    const items = Array.from(list.querySelectorAll<HTMLElement>(ITEM));
    const labels = items.map((item) => item.querySelector<HTMLElement>(LABEL));
    const cut = labels.map((label) => !!label && label.scrollWidth > label.clientWidth);
    items.forEach((item, i) => {
        const text = cut[i] ? (labels[i]?.textContent ?? "") : "";
        if (text) {
            if (item.getAttribute("data-hover-tip") !== text) {
                item.setAttribute("data-hover-tip", text);
                item.setAttribute("aria-description", text);
            }
        } else if (item.hasAttribute("data-hover-tip")) {
            item.removeAttribute("data-hover-tip");
            item.removeAttribute("aria-description");
        }
    });
}

/**
 * Keeps `list`'s cut labels marked: `schedule()` after the rows changed, and on its own when the list's width changes
 * (a window resized with the palette open cuts other labels, and whole ones that were cut). Several calls before the
 * next frame is drawn are one pass. Returns the stop.
 */
export function watchCutLabels(list: HTMLElement): { schedule: () => void; stop: () => void } {
    let cancel: (() => void) | undefined;
    const schedule = (): void => {
        if (cancel) return;
        cancel = afterNextPaint(() => {
            cancel = undefined;
            if (list.isConnected) markCutLabels(list);
        });
    };
    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined") {
        observer = new ResizeObserver(schedule);
        observer.observe(list);
    }
    return {
        schedule,
        stop: () => {
            cancel?.();
            cancel = undefined;
            observer?.disconnect();
        },
    };
}
