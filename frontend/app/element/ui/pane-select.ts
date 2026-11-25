// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A text selection that starts in a pane stays in that pane.
 *
 * Nothing in the page confines a document selection to a pane, so a press in one pane's text, held and
 * moved over the panes beside it, highlighted the text, cards and images of every pane it crossed. Window
 * chrome, other panes, the Swarm, the Kanban cards, the tab strips and the status bar are never part of a
 * selection that began in a pane. This holds however the selection is extended:
 *
 *  - a drag, also a double- or triple-click drag: while the primary button is held after a press in pane P,
 *    the root carries `data-pane-select` and P carries `data-pane-select-owner`, and the stylesheet makes
 *    everything outside P `user-select: none`. The engine never has anything outside P to extend to. While
 *    the pointer is over something that is not P (a field or a handle there makes the engine snap the end
 *    to the nearest position it can select, which can be before P in document order) the selection is held
 *    where the pointer last had it inside P. The attributes go on at the first move past `DRAG_SLOP_PX` that is over
 *    something other than the pane (a click, or a drag that stays inside the pane, sets nothing, so it never restyles
 *    the document), and off at the release, a cancel, a
 *    drag of the text out, the window losing focus or P leaving the page;
 *  - Shift+click: a press that would extend a selection from one pane into another drops the selection
 *    first, so the click starts a new one where it landed;
 *  - Shift+arrows, Shift+Home/End and the rest: the selection is put back inside P when its end leaves it
 *    (it stops at the pane's content edge);
 *  - Ctrl+A / Cmd+A outside a text field, the terminal and the Editor: selects the pane's own content.
 *
 * Text fields, the terminal and the Editor keep their own selections: a field's Select all stays inside the
 * field. Installed once at startup.
 */

import "./pane-select.scss";

export const ROOT_ATTR = "data-pane-select";
export const OWNER_ATTR = "data-pane-select-owner";

/** How far the pointer moves, in CSS px, before a press counts as a drag. */
export const DRAG_SLOP_PX = 4;

/** What a pane is: the frame every pane's view sits in. */
const PANE = "[data-blockid]";

/** The part of a pane that Select all takes. */
const PANE_BODY = ".block-content";

/** Where Select all is the widget's own: a text field, the terminal, the Editor, anything marked. */
const OWN_SELECT_ALL = "input, textarea, select, [contenteditable]:not([contenteditable=false]), .xterm, .monaco-editor, [data-pane-select-skip]";

/** The pane's hidden focus anchor: an input that is never typed into. */
const FOCUS_ANCHOR = ".dummy-focus";

function elementOf(node: Node | EventTarget | null): Element | null {
    if (!node) return null;
    if (node instanceof Element) return node;
    return (node as Node).parentElement ?? null;
}

/** The pane a node is inside, or null (window chrome, a portal, the status bar). */
export function paneOf(node: Node | EventTarget | null): Element | null {
    return elementOf(node)?.closest(PANE) ?? null;
}

function bodyOf(pane: Element): Element {
    return pane.querySelector(PANE_BODY) ?? pane;
}

function isMacPlatform(win: Window): boolean {
    return /Mac|iPhone|iPad/i.test(win.navigator.platform || win.navigator.userAgent || "");
}

interface Drag {
    pane: Element;
    /** Where the press was, to tell a drag from a click. */
    x: number;
    y: number;
    /** The attributes are on: the pointer has moved since the press. */
    marked: boolean;
    /** The pointer is over the pane (not over another pane, the chrome or the gap between). */
    inside: boolean;
    /** The selection as it stood the last time it changed with the pointer over the pane. */
    last: SelectionEnds | null;
}

type SelectionEnds = [Node, number, Node, number];

/**
 * A held drag whose pointer is over something that is not the pane: over a field or a handle (`user-select: none`)
 * the engine snaps the far end to the nearest position it can select, which can be the start of the document's
 * order before the pane, so the selection leaps backwards over other panes. It stays where it last was in the pane.
 */
function holdSelection(doc: Document, d: Drag): void {
    const sel = doc.getSelection();
    if (!sel || !sel.anchorNode || !sel.focusNode) return;
    const now: SelectionEnds = [sel.anchorNode, sel.anchorOffset, sel.focusNode, sel.focusOffset];
    if (d.inside || !d.last) {
        if (d.inside) d.last = now;
        return;
    }
    if (now.some((v, i) => v !== d.last![i])) sel.setBaseAndExtent(...d.last);
}

/** Whether a press on `el` can start no selection: the element, or what it inherits from, is `user-select: none`. */
export function cannotSelect(win: Window, el: Element | null): boolean {
    return !!el && win.getComputedStyle(el).userSelect === "none";
}

export function installPaneSelectGuard(win: Window = window): () => void {
    const doc = win.document;
    const mac = isMacPlatform(win);
    let drag: Drag | null = null;

    const end = (): void => {
        if (!drag) return;
        if (drag.marked) {
            drag.pane.removeAttribute(OWNER_ATTR);
            doc.documentElement.removeAttribute(ROOT_ATTR);
        }
        drag = null;
    };

    const mark = (d: Drag): void => {
        d.marked = true;
        d.pane.setAttribute(OWNER_ATTR, "");
        doc.documentElement.setAttribute(ROOT_ATTR, "");
    };

    /** Shift+click extends the selection from where it began to the click: never from one pane into another. */
    const dropCrossingSelection = (target: EventTarget | null): void => {
        const sel = doc.getSelection();
        if (!sel || sel.rangeCount === 0 || !sel.anchorNode) return;
        const anchorPane = paneOf(sel.anchorNode);
        if (anchorPane && anchorPane !== paneOf(target)) sel.removeAllRanges();
    };

    const onDown = (e: Event): void => {
        const p = e as PointerEvent;
        end();
        if (p.button !== 0 || (p.pointerType && p.pointerType !== "mouse" && p.pointerType !== "pen")) return;
        if (p.shiftKey) dropCrossingSelection(p.target);
        const pane = paneOf(p.target);
        if (!pane) return;
        // A press on something that says `user-select: none` (the Atlas map, a button, a header) cannot start a
        // selection, so there is nothing to confine, and marking it restyles every element of the window.
        if (cannotSelect(win, elementOf(p.target))) return;
        drag = { pane, x: p.clientX, y: p.clientY, marked: false, inside: true, last: null };
    };

    const onMove = (e: Event): void => {
        if (!drag) return;
        const p = e as PointerEvent;
        // The release happened where we could not see it, or the pane was closed under the drag.
        if (p.buttons === 0 || !drag.pane.isConnected) {
            end();
            return;
        }
        drag.inside = paneOf(p.target) === drag.pane;
        // Only a pointer outside the pane can reach another pane; marking restyles the whole window.
        if (!drag.marked && !drag.inside && Math.hypot(p.clientX - drag.x, p.clientY - drag.y) >= DRAG_SLOP_PX) mark(drag);
    };

    /** A selection that began in a pane and whose far end has left it: the end goes back to the pane's edge. */
    const keepInPane = (): void => {
        if (drag) return holdSelection(doc, drag); // a held press: the stylesheet confines it, the pointer moves the end
        const sel = doc.getSelection();
        if (!sel || sel.rangeCount === 0 || !sel.anchorNode || !sel.focusNode) return;
        const pane = paneOf(sel.anchorNode);
        if (!pane || pane.contains(sel.focusNode)) return;
        const body = bodyOf(pane);
        const forward = !!(sel.anchorNode.compareDocumentPosition(sel.focusNode) & Node.DOCUMENT_POSITION_FOLLOWING);
        sel.setBaseAndExtent(sel.anchorNode, sel.anchorOffset, body, forward ? body.childNodes.length : 0);
    };

    const onKeyDown = (e: Event): void => {
        const k = e as KeyboardEvent;
        if (k.defaultPrevented || k.altKey || k.shiftKey || k.key.toLowerCase() !== "a") return;
        if (mac ? !(k.metaKey && !k.ctrlKey) : !(k.ctrlKey && !k.metaKey)) return;
        const target = elementOf(k.target);
        if (target && !target.closest(FOCUS_ANCHOR) && target.closest(OWN_SELECT_ALL)) return;
        // With nothing focused the key lands on the body: it belongs to the pane that holds the caret.
        const pane = (target && paneOf(target)) ?? doc.querySelector(`${PANE}.block-focused`);
        if (!pane) return;
        const sel = doc.getSelection();
        if (!sel) return;
        k.preventDefault();
        sel.selectAllChildren(bodyOf(pane));
    };

    const listeners: Array<[string, EventListener, boolean | undefined]> = [
        ["pointerdown", onDown, true],
        ["pointerup", end, true],
        ["pointercancel", end, true],
        ["pointermove", onMove, true],
        // The text of a selection dragged out of the pane starts a drag of its own: no selecting any more.
        ["dragstart", end, true],
        ["keydown", onKeyDown, false],
        ["blur", end, undefined],
    ];
    for (const [type, fn, capture] of listeners) win.addEventListener(type, fn, capture);
    doc.addEventListener("selectionchange", keepInPane);
    return () => {
        end();
        for (const [type, fn, capture] of listeners) win.removeEventListener(type, fn, capture);
        doc.removeEventListener("selectionchange", keepInPane);
    };
}
