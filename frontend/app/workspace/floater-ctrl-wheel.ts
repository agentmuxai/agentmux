// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Ctrl+Wheel recovery for floating panes.
 *
 * A floater hosts its browser as a child HWND, and CEF consumes Ctrl+Wheel for
 * its own native page zoom before the renderer sees it — so no `wheel` event
 * with `ctrlKey` ever reaches the DOM in a floating window. Measured, not
 * assumed: a real mouse produced 22/22 ctrl-wheel events in the docked main
 * window and **0** in a floater, while plain scrolling produced 49 in the same
 * floater. See `docs/analysis/ANALYSIS_FLOATER_CTRL_SCROLL_ZOOM_2026_08_31.md`
 * §7 and `agentmux-cef/src/floater_wheel.rs`.
 *
 * The host intercepts `WM_MOUSEWHEEL` with `MK_CONTROL` and emits
 * `floater:ctrl-wheel`. This module turns that back into the DOM event that was
 * swallowed.
 *
 * **Why re-dispatch rather than call a zoom function.** Ctrl+Wheel zoom is not
 * implemented once — `term`, `armory`, `editor`, `swarm` and `warden` each
 * register their own capture-phase handler on their own view root, and each
 * writes `term:zoom` for its own block. Calling any one of them here would mean
 * duplicating that dispatch and re-deciding which view is in the floater.
 * Synthesising the event instead lets every one of those handlers run exactly
 * as it does when docked, so this file needs no knowledge of view types and
 * does not have to change when a new one is added.
 *
 * Untrusted events are sufficient: those handlers read only `ctrlKey` and
 * `deltaY`, then call `preventDefault()`/`stopPropagation()` and write meta.
 * None of them check `isTrusted`.
 */

import { listenEvent } from "@/app/platform/ipc";

type CtrlWheelPayload = {
    deltaY?: number;
    /** Physical (device) px, client-relative to the floater's top-level window. */
    clientXPhysical?: number;
    clientYPhysical?: number;
};

/** Where the event happened, in CSS px, or null if the host couldn't supply it. */
function resolvePoint(p: CtrlWheelPayload): { x: number; y: number } | null {
    if (typeof p.clientXPhysical !== "number" || typeof p.clientYPhysical !== "number") {
        return null;
    }
    const dpr = window.devicePixelRatio || 1;
    return { x: Math.round(p.clientXPhysical / dpr), y: Math.round(p.clientYPhysical / dpr) };
}

/**
 * Element to aim the synthesised event at.
 *
 * Uses the real cursor position when the host supplied it, because Ctrl+Wheel is
 * NOT uniform across a pane: agent shell sub-blocks (`AgentShellSubblock.tsx`)
 * and tool previews (`ToolBlock.tsx`) register their own independently scoped
 * handlers, and the pane header takes a different zoom path. Aiming everything
 * at the block centre would zoom the whole block no matter what was under the
 * cursor.
 *
 * Falls back to the block centre only when the point is missing or lands outside
 * the document — better a whole-block zoom than none.
 */
function resolveTarget(point: { x: number; y: number } | null): Element | null {
    if (point) {
        const hit = document.elementFromPoint(point.x, point.y);
        if (hit) return hit;
    }
    const block = document.querySelector("[data-blockid]");
    if (!block) return null;
    const r = block.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return null;
    const cx = Math.round(r.left + r.width / 2);
    const cy = Math.round(r.top + r.height / 2);
    return document.elementFromPoint(cx, cy) ?? block;
}

/**
 * Start listening for host-forwarded Ctrl+Wheel. Returns an unsubscribe fn.
 *
 * Safe to call in any window: the host only emits this event to `floating-*`
 * labels, so in the main window the listener simply never fires.
 */
export async function initFloaterCtrlWheel(): Promise<() => void> {
    return listenEvent<CtrlWheelPayload>("floater:ctrl-wheel", (payload) => {
        const deltaY = typeof payload?.deltaY === "number" ? payload.deltaY : 0;
        if (deltaY === 0) return;

        const point = resolvePoint(payload);
        const target = resolveTarget(point);
        if (!target) return;

        target.dispatchEvent(
            new WheelEvent("wheel", {
                ctrlKey: true,
                deltaY,
                deltaMode: 0, // DOM_DELTA_PIXEL, matching a real wheel event
                // Carry the position through as well: handlers that inspect
                // `clientX/Y` (or call `closest()` from `event.target`) then see
                // the same geometry a real wheel would have produced.
                clientX: point?.x ?? 0,
                clientY: point?.y ?? 0,
                bubbles: true,
                cancelable: true,
                composed: true,
            })
        );
    });
}
