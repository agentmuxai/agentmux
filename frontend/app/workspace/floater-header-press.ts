// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a press (or double-click) in a floating pane's header belongs to the window.
 *
 * The empty part of the header moves the floater (and a double-click on it maximizes it). A control in
 * the header keeps its own press: a button, link or field, and a pane tab pill, which is a click target
 * (switch tab) and a drag source (move the tab to another pane or window). The pane grip is a drag
 * handle first, so it moves the window like the rest of the header.
 */
const HEADER_SELECTOR = '[data-role="block-header"]';
const OWN_PRESS_SELECTOR = "button, a, input, select, textarea, [role='button'], .pane-tab";
const GRIP_SELECTOR = ".pane-grip";

export function isFloaterHeaderMovePress(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    if (!target.closest(HEADER_SELECTOR)) return false;
    return !target.closest(OWN_PRESS_SELECTOR) || target.closest(GRIP_SELECTOR) !== null;
}
