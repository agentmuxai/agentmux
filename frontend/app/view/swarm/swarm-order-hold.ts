// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm's order, wired to the list on screen: the controller (swarm-order.ts), the
 * slide when rows move (swarm-order-motion.ts), and what holds the list still while it is
 * being used. Which pane has focus, and the ticked checkboxes, do not hold it.
 * docs/specs/SPEC_SWARM_ORDER_WORKING_FIRST_THEN_RECENT_2026_10_23.md §4.
 */

import { createComputed, onCleanup, onMount, type Accessor } from "solid-js";
import { SwarmOrderController, type SwarmOrderItem } from "./swarm-order";
import { captureRowTops, playRowMoves } from "./swarm-order-motion";

export interface SwarmOrder {
    /** The keys of the rows, in the order to show them. */
    order: Accessor<string[]>;
    /** Lets go of the holds the list element gave, for when it is removed. */
    releaseListHolds: () => void;
    /** Event handlers for the list element, which hold the order still. */
    listHandlers: {
        onPointerEnter: () => void;
        onPointerLeave: () => void;
        onPointerDown: () => void;
        onContextMenu: () => void;
        onFocusIn: () => void;
        onFocusOut: (e: FocusEvent) => void;
    };
}

/**
 * Orders the rows `items` describes. `list` is the list element, for the slide and for
 * telling whether focus left it; `zoom` is the pane's zoom.
 */
export function useSwarmOrder(opts: {
    items: Accessor<readonly SwarmOrderItem[]>;
    list: () => HTMLElement | undefined;
    zoom: Accessor<number>;
}): SwarmOrder {
    let tops = new Map<string, number>();
    const control = new SwarmOrderController(undefined, undefined, {
        beforeMove: () => {
            tops = captureRowTops(opts.list());
        },
        afterMove: () => playRowMoves(opts.list(), tops, opts.zoom()),
        stillHeld: (hold) => {
            const list = opts.list();
            if (!list) return false;
            return hold === "pointer" ? list.matches(":hover") : list.contains(document.activeElement);
        },
    });
    onCleanup(() => control.dispose());
    createComputed(() => control.update(opts.items()));

    // A button is down until it comes up anywhere, and a right-click menu is open until the
    // next press or key.
    onMount(() => {
        const release = () => control.setHold("press", false);
        const closeMenu = () => control.setHold("menu", false);
        const on = [
            ["pointerup", release],
            ["pointercancel", release],
            ["dragend", release],
            ["blur", release],
            ["pointerdown", closeMenu],
            ["keydown", closeMenu],
        ] as const;
        for (const [type, handler] of on) window.addEventListener(type, handler, true);
        onCleanup(() => {
            for (const [type, handler] of on) window.removeEventListener(type, handler, true);
        });
    });

    return {
        order: control.order,
        releaseListHolds: () => control.releaseListHolds(),
        listHandlers: {
            onPointerEnter: () => control.setHold("pointer", true),
            onPointerLeave: () => control.setHold("pointer", false),
            onPointerDown: () => control.setHold("press", true),
            // The native menu can swallow the pointerup, so a right-click ends the press.
            onContextMenu: () => {
                control.setHold("press", false);
                control.setHold("menu", true);
            },
            onFocusIn: () => control.setHold("focus", true),
            onFocusOut: (e) => {
                if (!opts.list()?.contains(e.relatedTarget as Node | null)) control.setHold("focus", false);
            },
        },
    };
}
