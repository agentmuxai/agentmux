// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the status bar shows: AgentMux's own items and the ones widgets add
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.8). Each item
 * renders itself; the bar lays them out by side and order.
 */

import { createSignal, type JSX } from "solid-js";

export interface StatusBarItem {
    /** Unique; registering an id again replaces the item. */
    id: string;
    side: "left" | "right";
    /** Lower first. AgentMux's own items use multiples of 100; widget items
     *  sit between `WIDGET_ORDER_RIGHT` / `WIDGET_ORDER_LEFT` and the next. */
    order: number;
    render: () => JSX.Element;
}

/** Widget items on the right come before AgentMux's own; on the left, after. */
export const WIDGET_ORDER_RIGHT = 50;
export const WIDGET_ORDER_LEFT = 1000;

const items = new Map<string, StatusBarItem>();
const [version, setVersion] = createSignal(0);

/** Adds `item`; returns what removes it (only while it is still the item
 *  registered under its id). */
export function registerStatusBarItem(item: StatusBarItem): () => void {
    items.set(item.id, item);
    setVersion((v) => v + 1);
    return () => {
        if (items.get(item.id) !== item) return;
        items.delete(item.id);
        setVersion((v) => v + 1);
    };
}

/** The items on `side`, in order. Reactive. */
export function statusBarItems(side: "left" | "right"): StatusBarItem[] {
    version();
    return [...items.values()].filter((i) => i.side === side).sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
}
