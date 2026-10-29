// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One ResizeObserver per window for every native browser pane's placeholder.
 *
 * Its callback runs in the rendering step that laid the panes out, with every
 * changed placeholder's entry at once. Each pane records its new rect, then
 * the window's batch goes out right there (`flushNow`), instead of in a task
 * queued behind whatever else a busy page has to run: the rect path no
 * longer waits for the next free task slot after the layout it reports.
 */

interface WindowObserver {
    observer: ResizeObserver;
    onChange: Map<Element, () => void>;
}

const observers = new Map<string, WindowObserver>();

/**
 * Calls `onChange` whenever `el`'s size changes, then `afterChanges` once all
 * of the changes the window's observer delivered together have been handled.
 * Returns the function that stops observing.
 */
export function observePaneElement(
    windowLabel: string,
    el: Element,
    onChange: () => void,
    afterChanges: () => void,
): () => void {
    let w = observers.get(windowLabel);
    if (!w) {
        const onChangeByElement = new Map<Element, () => void>();
        const observer = new ResizeObserver((entries) => {
            for (const entry of entries) onChangeByElement.get(entry.target)?.();
            afterChanges();
        });
        w = { observer, onChange: onChangeByElement };
        observers.set(windowLabel, w);
    }
    w.onChange.set(el, onChange);
    w.observer.observe(el);
    const own = w;
    return () => {
        own.onChange.delete(el);
        own.observer.unobserve?.(el);
        if (own.onChange.size === 0) {
            own.observer.disconnect();
            if (observers.get(windowLabel) === own) observers.delete(windowLabel);
        }
    };
}
