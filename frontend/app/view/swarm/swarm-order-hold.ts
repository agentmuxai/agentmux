// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm's order, wired to the list on screen: the controller (swarm-order.ts), the
 * slide when rows move (swarm-order-motion.ts), and what holds the list still while it is
 * being used. Which pane has focus, and the ticked checkboxes, do not hold it.
 * docs/specs/SPEC_SWARM_ORDER_WORKING_FIRST_THEN_RECENT_2026_10_23.md §4.
 */

import { createComputed, onCleanup, onMount, type Accessor } from "solid-js";
import type { Snapshot } from "@/app/motion/primitives";
import { SwarmOrderController, type SwarmOrderItem } from "./swarm-order";
import { captureRows, playRowMoves } from "./swarm-order-motion";

export interface SwarmOrder {
    /** The keys of the rows, in the order to show them. */
    order: Accessor<string[]>;
    /**
     * Call from the list element's ref callback. A right-click menu stops the event before it
     * bubbles, so the menu hold listens in the capture phase; and the list's holds end with it.
     */
    bindList: (list: HTMLElement) => void;
    /** Event handlers for the list element, which hold the order still. */
    listHandlers: {
        onPointerEnter: () => void;
        onPointerLeave: () => void;
        onPointerDown: () => void;
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
    let before: Snapshot = new Map();
    const control = new SwarmOrderController(undefined, undefined, {
        beforeMove: () => {
            before = captureRows(opts.list());
        },
        afterMove: () => playRowMoves(opts.list(), before, opts.zoom()),
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
            // Only the window losing focus: an element losing it does not reach this far unless captured.
            ["blur", (e: Event) => e.target === window && release()],
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
        bindList: (list) => {
            // The native menu can swallow the pointerup, so a right-click ends the press.
            const onMenu = () => {
                control.setHold("press", false);
                control.setHold("menu", true);
            };
            list.addEventListener("contextmenu", onMenu, true);
            // The pointer and focus holds end with the list, if its events never come.
            onCleanup(() => {
                list.removeEventListener("contextmenu", onMenu, true);
                control.releaseListHolds();
            });
        },
        listHandlers: {
            onPointerEnter: () => control.setHold("pointer", true),
            onPointerLeave: () => control.setHold("pointer", false),
            onPointerDown: () => control.setHold("press", true),
            onFocusIn: () => control.setHold("focus", true),
            onFocusOut: (e) => {
                if (!opts.list()?.contains(e.relatedTarget as Node | null)) control.setHold("focus", false);
            },
        },
    };
}
