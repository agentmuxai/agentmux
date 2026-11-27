// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A text selection made by dragging in a text field keeps extending, and stays highlighted, for as long as
 * the button is held, wherever the pointer goes (left of the field, over another pane, out of the window).
 *
 * The browser does this on its own. What breaks it is the page around the field, and two things were seen
 * in a real browser (the field inside a pane that carries `draggable="true"`, and an element the pointer
 * crosses calling `focus()` on another one):
 *
 *  - a draggable ancestor starts its own drag, and the selection stops at the press;
 *  - something under the pointer reacts to it (`mouseenter`, a hover card, a `click` that lands on a pane
 *    or row) and moves the focus away, so the field's highlight goes grey or the caret is gone.
 *
 * So after a press in a text field, while the primary button is held, this guard
 *
 *  - stops a `dragstart` that comes from an ancestor of the field (the field's own text drag, moving a
 *    selection, is left alone), from the press on;
 *  - puts the focus and the selection back if something took the focus off the field anyway, from the
 *    press on;
 *  - once the pointer has moved `DRAG_SLOP_PX` or more (a drag, not a click), marks the field
 *    `data-text-drag-field` and has the field capture the pointer: nothing else is hovered, nothing receives the
 *    release as a click, and so nothing reacts. A click sets nothing, and nothing in the document is restyled.
 *
 * The engine itself mis-maps a pointer that has left a field whose ancestor carries a CSS transform (the
 * pane tiles do: `translate3d(x, y, 0)`): it reads the pointer's page x as if it were x inside the field, so
 * the selection collapses and then lands on a character that has nothing to do with where the pointer is.
 * While the pointer is outside the field's box the guard therefore asks the engine for the caret at the
 * pointer pulled into the box (`caretPositionFromPoint`, which maps correctly) and sets the selection from
 * the press's anchor to it.
 *
 * It covers every `<input>` and `<textarea>`, kit or not, and ends on release, cancel, a move with no
 * button down, the window losing focus or the field leaving the page. Installed once at startup.
 */

const FIELD_ATTR = "data-text-drag-field";

/** Input types that hold text a person selects by dragging. */
const TEXT_TYPES = new Set(["", "text", "search", "email", "url", "tel", "password", "number"]);

/** Fields that are not ours to guard: the terminal's hidden input, and anything marked. */
const SKIP = ".xterm, [data-text-drag-skip]";

import { beginSelectDrag, endSelectDrag } from "./drag-lag";

/** The text field a press landed on, or null. */
export function textFieldOf(target: EventTarget | null): HTMLInputElement | HTMLTextAreaElement | null {
    if (!(target instanceof Element)) return null;
    const field = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement ? target : null;
    if (!field || field.closest(SKIP)) return null;
    if (field instanceof HTMLInputElement && !TEXT_TYPES.has(field.type)) return null;
    return field;
}

/** How far the pointer moves, in CSS px, before a press in a field counts as a drag. */
export const DRAG_SLOP_PX = 4;

interface Drag {
    field: HTMLInputElement | HTMLTextAreaElement;
    /** Where the press was, to tell a drag from a click. */
    x: number;
    y: number;
    /** The attributes are on: the pointer has moved since the press. */
    marked: boolean;
    /** The pointer the press came from: the field captures it once the drag is marked. */
    pointerId: number;
    /** The field's box, read once in the drag (the page does not move under a held press), or null before. */
    box: DOMRect | null;
    /** The fixed end of the selection, known once the drag is marked. Null for a word or line drag. */
    anchor: number | null;
    /** Where the pointer is now. */
    at: { x: number; y: number };
    /** The press was a double or triple click: the engine selects by word or line and keeps doing so. */
    multi: boolean;
    /** Where the selection was when the focus left, to put back. */
    lost: { start: number; end: number; dir: "forward" | "backward" | "none" } | null;
}

type Field = HTMLInputElement | HTMLTextAreaElement;

/** The caret offset the engine gives for the point (x, y) pulled just inside the field's box, or null. */
export function caretInField(doc: Document, field: Field, x: number, y: number, box?: DOMRect): number | null {
    const r = box ?? field.getBoundingClientRect();
    if (r.width < 3 || r.height < 3) return null;
    const probe = (doc as Document & { caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null })
        .caretPositionFromPoint;
    const c = probe?.call(doc, Math.min(Math.max(x, r.left + 1), r.right - 1), Math.min(Math.max(y, r.top + 1), r.bottom - 1));
    return c && c.offsetNode === field ? c.offset : null;
}

/** The selection the pointer at (x, y) means, from `anchor`, while the pointer is outside the field's box. */
function selectionToward(doc: Document, field: Field, anchor: number, x: number, y: number, box?: DOMRect) {
    const r = box ?? field.getBoundingClientRect();
    if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return null;
    const at = caretInField(doc, field, x, y, r);
    if (at === null) return null;
    return { start: Math.min(anchor, at), end: Math.max(anchor, at), dir: at < anchor ? ("backward" as const) : ("forward" as const) };
}

/** The end of the selection that stays put while the pointer drags the other. */
function anchorOf(field: Field): number {
    return (field.selectionDirection === "backward" ? field.selectionEnd : field.selectionStart) ?? 0;
}

// The attributes go on at the first move, not at the press. The old root attribute restyled the
// whole document (5,000-plus elements, about 45 ms on a six-pane layout; 200 to 300 ms on long transcripts), and setting it at the
// press and clearing it at the release did that twice for every click into a composer, which is why
// clicking into an input box lagged where clicking a pane's body did not (#45977). A click never
// moves; nothing else is under the pointer until it leaves the field, and the `dragstart` stop and
// the focus restore work from the press alone.
function markDrag(doc: Document, d: Drag): void {
    d.marked = true;
    beginSelectDrag();
    d.anchor = anchorOf(d.field);
    d.field.setAttribute(FIELD_ATTR, "");
    // The field keeps the pointer: nothing else is hovered and the release lands on the field, not on a pane or row, with no
    // rule that restyles the page (the old root attribute restyled all of it at this first move).
    try {
        d.field.setPointerCapture(d.pointerId);
    } catch {
        // A pointer that has already gone (a release raced the move): nothing to hold.
    }
}

function restoreFocus(d: Drag): void {
    if (!d.lost || !d.field.isConnected) return;
    const { start, end, dir } = d.lost;
    d.lost = null;
    d.field.focus({ preventScroll: true });
    try {
        d.field.setSelectionRange(start, end, dir);
    } catch {
        // A field type with no selection (number) keeps the focus only.
    }
}

/**
 * Past the field's edge the engine maps the pointer wrongly under a transformed ancestor: set the selection
 * from where the pointer is. Run after every move and again when the engine has set its own.
 */
function followPointer(doc: Document, d: Drag): void {
    if (!d.marked || d.anchor === null || d.multi || !d.field.isConnected) return;
    // The box is read once for the drag, on the first move that needs it: a read on every move laid out a page the drag had just restyled.
    d.box ??= d.field.getBoundingClientRect();
    const sel = selectionToward(doc, d.field, d.anchor, d.at.x, d.at.y, d.box);
    const f = d.field;
    if (!sel || (f.selectionStart === sel.start && f.selectionEnd === sel.end && f.selectionDirection === sel.dir)) return;
    try {
        f.setSelectionRange(sel.start, sel.end, sel.dir);
    } catch {
        // A field type with no selection (number) is left to the engine.
    }
}

export function installTextDragGuard(win: Window = window): () => void {
    const doc = win.document;
    let drag: Drag | null = null;

    const end = (): void => {
        if (!drag) return;
        if (drag.marked) {
            endSelectDrag();
            drag.field.removeAttribute(FIELD_ATTR);
            if (drag.field.hasPointerCapture?.(drag.pointerId)) drag.field.releasePointerCapture(drag.pointerId);
        }
        drag = null;
    };

    const onDown = (e: Event): void => {
        const p = e as PointerEvent;
        end();
        if (p.button !== 0 || (p.pointerType && p.pointerType !== "mouse" && p.pointerType !== "pen")) return;
        const field = textFieldOf(p.target);
        if (!field) return;
        drag = { field, x: p.clientX, y: p.clientY, marked: false, pointerId: p.pointerId, box: null, anchor: null, at: { x: p.clientX, y: p.clientY }, multi: false, lost: null };
    };

    // A double or triple click selects by word or line: the engine keeps that granularity, so leave it be.
    const onMouseDown = (e: Event): void => {
        if (drag && (e as MouseEvent).detail > 1) drag.multi = true;
    };

    const follow = (): void => {
        if (drag) followPointer(doc, drag);
    };

    const onDragStart = (e: Event): void => {
        const t = e.target;
        // An ancestor's drag (a pane, a tab, a row) must not take over the press. The field's own text drag
        // has the field or its text as the target.
        if (drag && t instanceof Element && t !== drag.field && t.contains(drag.field)) {
            e.preventDefault();
            e.stopImmediatePropagation();
        }
    };

    const onFocusOut = (e: Event): void => {
        const d = drag;
        if (!d || e.target !== d.field) return;
        const related = (e as FocusEvent).relatedTarget;
        // Focus leaving the page (a window switch) is not a theft.
        if (!related) return;
        d.lost = {
            start: d.field.selectionStart ?? 0,
            end: d.field.selectionEnd ?? 0,
            dir: d.field.selectionDirection ?? "none",
        };
        win.requestAnimationFrame(() => {
            if (drag === d) restoreFocus(d);
        });
    };

    const onMove = (e: Event): void => {
        if (!drag) return;
        const p = e as PointerEvent;
        // The release happened where we could not see it, or the field was replaced under the drag.
        if (p.buttons === 0 || !drag.field.isConnected) {
            end();
            return;
        }
        drag.at = { x: p.clientX, y: p.clientY };
        if (!drag.marked && Math.hypot(p.clientX - drag.x, p.clientY - drag.y) >= DRAG_SLOP_PX) markDrag(doc, drag);
        follow();
    };

    const onUp = (e: Event): void => {
        if (drag) drag.at = { x: (e as PointerEvent).clientX, y: (e as PointerEvent).clientY };
        follow();
        end();
    };

    const listeners: Array<[string, EventListener, boolean | undefined]> = [
        ["pointerdown", onDown, true],
        ["pointerup", onUp, true],
        ["mousedown", onMouseDown, true],
        ["pointercancel", end, true],
        ["pointermove", onMove, true],
        ["dragstart", onDragStart, true],
        ["focusout", onFocusOut, true],
        ["blur", end, undefined],
    ];
    doc.addEventListener("selectionchange", follow);
    for (const [type, fn, capture] of listeners) win.addEventListener(type, fn, capture);
    return () => {
        end();
        doc.removeEventListener("selectionchange", follow);
        for (const [type, fn, capture] of listeners) win.removeEventListener(type, fn, capture);
    };
}
