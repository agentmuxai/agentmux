// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Chrome's address bar selection. When focus arrives from the keyboard (Tab,
 * the focus-address shortcut), the whole address is selected at once. When it
 * arrives with a click, the click decides on mouseup, after the browser has
 * placed the caret or a drag selection:
 *
 * - a plain click that moved focus into the bar selects the whole address, so
 *   typing replaces it;
 * - a drag on that click keeps the text it selected;
 * - a click while the bar already has focus places the caret, and double and
 *   triple clicks keep their usual meaning.
 */
export function createFirstClickSelectAll(): {
    onMouseDown: (input: HTMLInputElement) => void;
    onFocus: (input: HTMLInputElement) => void;
    onMouseUp: (input: HTMLInputElement) => void;
} {
    let pressing = false;
    let firstClick = false;
    return {
        onMouseDown(input) {
            pressing = true;
            // A drag can end outside the bar, where its own mouseup never fires.
            document.addEventListener("mouseup", () => (pressing = false), { once: true, capture: true });
            // The bar counts as focused only while its window has focus too:
            // after a click into the page (a native window), activeElement is
            // still the input but the document no longer has focus.
            firstClick = document.activeElement !== input || !document.hasFocus();
        },
        onFocus(input) {
            // Selecting during a press would turn a drag into "select all".
            if (!pressing) input.select();
        },
        onMouseUp(input) {
            pressing = false;
            if (!firstClick) return;
            firstClick = false;
            if (input.selectionStart === input.selectionEnd) input.select();
        },
    };
}
