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

type CtrlWheelPayload = { deltaY?: number };

/** Element to aim the synthesised event at. */
function resolveTarget(): Element | null {
    // A floater wraps exactly one block, so its content is the only sensible
    // target. Aim at the deepest element at the block's centre so the event
    // propagates through the view root that owns the capture-phase handler,
    // rather than being dispatched directly on the root (which would skip
    // nothing, but would misreport `event.target` to anything inspecting it).
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

        const target = resolveTarget();
        if (!target) return;

        target.dispatchEvent(
            new WheelEvent("wheel", {
                ctrlKey: true,
                deltaY,
                deltaMode: 0, // DOM_DELTA_PIXEL, matching a real wheel event
                bubbles: true,
                cancelable: true,
                composed: true,
            })
        );
    });
}
