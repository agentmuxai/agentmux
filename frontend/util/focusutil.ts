// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0s

import * as util from "./util";

function findBlockId(element: HTMLElement): string | null {
    let current: HTMLElement = element;
    while (current) {
        if (current.hasAttribute("data-blockid")) {
            return current.getAttribute("data-blockid");
        }
        current = current.parentElement;
    }
    return null;
}

export function getElemAsStr(elem: EventTarget) {
    if (elem == null) {
        return "null";
    }
    if (!(elem instanceof HTMLElement)) {
        if (elem instanceof Text) {
            elem = elem.parentElement;
        }
        if (!(elem instanceof HTMLElement)) {
            return "unknown";
        }
    }
    const blockId = findBlockId(elem);
    let rtn = elem.tagName.toLowerCase();
    if (!util.isBlank(elem.id)) {
        rtn += "#" + elem.id;
    }
    if (!util.isBlank(elem.className)) {
        rtn += "." + elem.className;
    }
    if (blockId != null) {
        rtn += ` [${blockId.substring(0, 8)}]`;
    }
    return rtn;
}

/** `<input type=…>` values that don't take typed text — clicking one is not
 *  "the user chose where to type". */
const NON_TEXT_INPUT_TYPES = new Set([
    "button",
    "checkbox",
    "color",
    "file",
    "hidden",
    "image",
    "radio",
    "range",
    "reset",
    "submit",
]);

/**
 * True when the caret is already in a real text-entry control inside
 * `blockId` — the user (or the view) chose where to type, and pane selection
 * must not override that choice. The ONE guard every pane-selection focus
 * path consults (focusManager.refocusNode / claimFocusOnMount,
 * block-component-registry's refocusNode, block.tsx's click handler) — see
 * SPEC_PANE_CLICK_THROUGH_INPUT_FOCUS_2026_09_23.md §4.1.
 *
 * Deliberately reads `document.activeElement` only, never the text-selection
 * fallback `focusedBlockId()` uses: "text selected in B, caret in A" must not
 * read as "the user is typing in B".
 */
export function userCaretInBlock(blockId: string): boolean {
    const el = document.activeElement;
    if (!(el instanceof HTMLElement) || el.classList.contains("dummy-focus")) return false;
    if (findBlockId(el) !== blockId) return false;
    if (el instanceof HTMLInputElement) return !NON_TEXT_INPUT_TYPES.has(el.type);
    return el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement || el.isContentEditable === true;
}

/**
 * For a document-level listener owned by something inside a pane (a popup,
 * an overlay, a prompt): does event `e` belong to the pane containing `el`?
 * True when `e` originated inside that pane, or — when its target is outside
 * every pane (e.g. `body`, nothing focused) — when that pane is the selected
 * one. Without this, the same popup open in two side-by-side panes both react
 * to one keystroke, or a popup in pane A reacts to typing in pane B.
 * An `el` that isn't inside any pane is app-global: always true.
 */
export function eventBelongsToPaneOf(e: Event, el: Element | null | undefined): boolean {
    if (!(el instanceof HTMLElement)) return true;
    const paneId = findBlockId(el);
    if (paneId == null) return true;
    const targetPane = e.target instanceof HTMLElement ? findBlockId(e.target) : null;
    if (targetPane != null) return targetPane === paneId;
    return el.closest(".block-focused") != null;
}

export function focusedBlockId(): string {
    const focused = document.activeElement;
    if (focused instanceof HTMLElement) {
        const blockId = findBlockId(focused);
        if (blockId) {
            return blockId;
        }
    }
    const sel = document.getSelection();
    if (sel && sel.anchorNode && sel.rangeCount > 0 && !sel.isCollapsed) {
        let anchor = sel.anchorNode;
        if (anchor instanceof Text) {
            anchor = anchor.parentElement;
        }
        if (anchor instanceof HTMLElement) {
            const blockId = findBlockId(anchor);
            if (blockId) {
                return blockId;
            }
        }
    }
    return null;
}
