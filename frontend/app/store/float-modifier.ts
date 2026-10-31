// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Ctrl+click (Cmd+click on macOS) on anything that opens a pane opens the pane floating.
 *
 * One rule, in one place, instead of a modifier check in every click handler: a capture listener notes a
 * click made with the modifier, and the code that opens a pane (`createBlock`, `openOrFocusPaneIdByView`,
 * the Editor openers, link clicks) asks `takeFloatRequest()`. The first to ask within a moment gets a
 * yes, and no other open does. A click that opens nothing (Files' multi-select, the image viewer's
 * zoom-out) leaves the note to lapse, and the next plain click or press clears it.
 *
 * Floating is a native window, so a host without multiple windows ignores the modifier.
 */

import { hostHas } from "@/app/host/host-caps";
import { isModKey } from "@/util/platformutil";

/** How long after the click a pane open still counts as its result. A handler may await an RPC first. */
const NOTE_MS = 600;

let noteUntil = 0;

/** Whether this mouse event is a "float it" click: the platform's command key alone. */
export function isFloatClick(e: Pick<MouseEvent, "ctrlKey" | "metaKey" | "shiftKey" | "altKey">): boolean {
    return isModKey(e) && !e.shiftKey && !e.altKey;
}

/** Note (or clear) the request from a click. */
export function noteClick(e: Pick<MouseEvent, "ctrlKey" | "metaKey" | "shiftKey" | "altKey">, now = Date.now()): void {
    noteUntil = isFloatClick(e) ? now + NOTE_MS : 0;
}

/** True once for the pane open that a modifier-click caused; false for every other open. */
export function takeFloatRequest(now = Date.now()): boolean {
    if (now >= noteUntil) return false;
    if (!hostHas("multiWindow")) return false;
    noteUntil = 0;
    return true;
}

/** Listen for the clicks, on this window. Call once at startup. */
export function installFloatModifier(target: Pick<Window, "addEventListener"> = window): void {
    // Capture: before any handler of the click can open a pane.
    target.addEventListener("click", (e) => noteClick(e as MouseEvent), true);
    target.addEventListener("dblclick", (e) => noteClick(e as MouseEvent), true);
    // A new press without the modifier starts over; with it, the click that follows notes it.
    target.addEventListener("pointerdown", (e) => {
        if (!isFloatClick(e as PointerEvent)) noteUntil = 0;
    }, true);
    target.addEventListener("keydown", (e) => {
        const k = e as KeyboardEvent;
        if (k.key !== "Control" && k.key !== "Meta") noteUntil = 0;
    }, true);
}
