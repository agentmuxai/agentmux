// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The hover bridge: the grace period that keeps a tall panel up while the pointer crosses to it. */

import { createComputed, on, onCleanup, untrack } from "solid-js";
import {
    beginPeekBridge,
    clearPeekPanelOpenAfterLeave,
    endPeekBridge,
    markPeekPanelOpenAfterLeave,
} from "./peek-bridge";
import type { PeekState } from "./peek-overlay-state";

/**
 * How long an enterable panel lingers after the pointer leaves its row, so the
 * pointer can reach it. Re-armed on every move that brings the pointer closer
 * to the panel, so a slow or diagonal approach still makes it; a pointer moving
 * away lets it close after this long. Entering the panel then holds it open
 * (scroll bar, text selection). Standard hover-card grace.
 */
const HOVER_BRIDGE_MS = 150;
/** The longest an approaching pointer can keep the grace period going, in total. */
const HOVER_BRIDGE_MAX_MS = 1000;

/** The bridge's timer and the pointer tracking during it. Makes no reactive computation. */
export function createPeekBridge(s: PeekState) {
    const releaseBridge = () => {
        endPeekBridge(s.bridgedRow);
        s.bridgedRow = undefined;
    };

    // How far the pointer is from the panel (0 inside it).
    const distanceToPanel = (x: number, y: number): number => {
        const r = s.floatingEl?.getBoundingClientRect();
        if (!r) return Infinity;
        const dx = Math.max(r.left - x, 0, x - r.right);
        const dy = Math.max(r.top - y, 0, y - r.bottom);
        return Math.hypot(dx, dy);
    };
    // During the grace period: a move that brings the pointer closer than it has
    // been re-arms the timer (up to HOVER_BRIDGE_MAX_MS in total).
    const onApproach = (e: MouseEvent) => {
        const d = distanceToPanel(e.clientX, e.clientY);
        if (d < s.closestApproach) {
            s.closestApproach = d;
            armBridgeTimer();
        }
    };
    const clearBridgeTimer = () => {
        if (s.bridgeTimer !== undefined) clearTimeout(s.bridgeTimer);
        s.bridgeTimer = undefined;
        document.removeEventListener("mousemove", onApproach);
    };
    const armBridgeTimer = () => {
        if (s.bridgeTimer !== undefined) clearTimeout(s.bridgeTimer);
        const budget = HOVER_BRIDGE_MAX_MS - (Date.now() - s.bridgeStartedAt);
        s.bridgeTimer = setTimeout(endLinger, Math.max(0, Math.min(HOVER_BRIDGE_MS, budget)));
    };
    const endLinger = () => {
        clearBridgeTimer();
        if (!s.held && !s.show()) {
            releaseBridge();
            s.setOpen(false);
        }
    };
    const startLinger = () => {
        clearBridgeTimer();
        s.bridgeStartedAt = Date.now();
        s.closestApproach = Infinity;
        // Other rows' peeks wait while the pointer crosses to this panel.
        s.bridgedRow = s.props.rowEl();
        beginPeekBridge(s.bridgedRow);
        document.addEventListener("mousemove", onApproach, { passive: true });
        armBridgeTimer();
    };

    return { releaseBridge, clearBridgeTimer, startLinger };
}

export type PeekBridge = ReturnType<typeof createPeekBridge>;

/**
 * Creates two computations and their cleanups, in this order: `open` follows `show` (with the linger), then
 * the open-after-leave marker follows `open`.
 */
export function usePeekLinger(s: PeekState, b: PeekBridge) {
    // Hover bridge. The panel is portalled, so moving onto it
    // fires the row's `mouseleave` and the caller turns `show` off. Keep it up
    // for HOVER_BRIDGE_MS after that, and while the pointer is on it, so the
    // pointer can cross the gap and use the scroll bar / select text. The
    // caller's `show` stays authoritative: when it goes true again (pointer back
    // on the row) the linger is cancelled.
    createComputed(
        on(
            () => s.show(),
            (show) => {
                b.clearBridgeTimer();
                if (show) {
                    // The pointer is back on the row: nothing is being crossed.
                    b.releaseBridge();
                    s.setOpen(true);
                } else if (untrack(s.open) && s.lastEnterable) {
                    // Only a panel that had to be cut (it scrolls) is meant to be
                    // ENTERED, so only it lingers. A panel that fits has nothing
                    // to scroll and closes the instant the pointer leaves, exactly
                    // as it always did.
                    b.startLinger(); // stay open; the timer closes it
                } else {
                    s.setOpen(false);
                }
            },
            { defer: true },
        ),
    );
    onCleanup(b.clearBridgeTimer);

    // The panel is on screen although its row is no longer hovered (lingering, or
    // the pointer on it). The caller's `isPeeking()` is false then, so tell it,
    // through peek-bridge.ts, that the content must stay: callers gate content on
    // `useNodePeek().panelVisible()`. The row is remembered so the same one is
    // cleared even if the caller's row element changes meanwhile.
    createComputed(() => {
        const openAfterLeave = s.open() && !s.show();
        untrack(() => {
            if (openAfterLeave) {
                s.markedRow = s.props.rowEl();
                markPeekPanelOpenAfterLeave(s.markedRow);
            } else if (s.markedRow) {
                clearPeekPanelOpenAfterLeave(s.markedRow);
                s.markedRow = undefined;
            }
        });
    });
    onCleanup(() => {
        clearPeekPanelOpenAfterLeave(s.markedRow);
        s.markedRow = undefined;
    });
}
