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
 *    `data-text-drag-field` and the root `data-text-drag`, which the kit stylesheet turns into
 *    `pointer-events: none` for everything else: nothing is hovered, nothing receives the release as a
 *    click, and so nothing reacts. A click sets nothing: the root attribute restyles the whole document.
 *
 * It covers every `<input>` and `<textarea>`, kit or not, and ends on release, cancel, a move with no
 * button down, the window losing focus or the field leaving the page. Installed once at startup.
 */

const ROOT_ATTR = "data-text-drag";
const FIELD_ATTR = "data-text-drag-field";

/** Input types that hold text a person selects by dragging. */
const TEXT_TYPES = new Set(["", "text", "search", "email", "url", "tel", "password", "number"]);

/** Fields that are not ours to guard: the terminal's hidden input, and anything marked. */
const SKIP = ".xterm, [data-text-drag-skip]";

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
    /** Where the selection was when the focus left, to put back. */
    lost: { start: number; end: number; dir: "forward" | "backward" | "none" } | null;
}

export function installTextDragGuard(win: Window = window): () => void {
    const doc = win.document;
    let drag: Drag | null = null;

    const end = (): void => {
        if (!drag) return;
        if (drag.marked) {
            drag.field.removeAttribute(FIELD_ATTR);
            doc.documentElement.removeAttribute(ROOT_ATTR);
        }
        drag = null;
    };

    // The attributes go on at the first move, not at the press. `html[data-text-drag] *` restyles the
    // whole document (5,000-plus elements, about 45 ms on a six-pane layout), and setting it at the
    // press and clearing it at the release did that twice for every click into a composer, which is why
    // clicking into an input box lagged where clicking a pane's body did not (#45977). A click never
    // moves; nothing else is under the pointer until it leaves the field, and the `dragstart` stop and
    // the focus restore below work from the press alone.
    const mark = (d: Drag): void => {
        d.marked = true;
        d.field.setAttribute(FIELD_ATTR, "");
        doc.documentElement.setAttribute(ROOT_ATTR, "");
    };

    const restoreFocus = (d: Drag): void => {
        if (drag !== d || !d.lost || !d.field.isConnected) return;
        const { start, end: stop, dir } = d.lost;
        d.lost = null;
        d.field.focus({ preventScroll: true });
        try {
            d.field.setSelectionRange(start, stop, dir);
        } catch {
            // A field type with no selection (number) keeps the focus only.
        }
    };

    const onDown = (e: Event): void => {
        const p = e as PointerEvent;
        end();
        if (p.button !== 0 || (p.pointerType && p.pointerType !== "mouse" && p.pointerType !== "pen")) return;
        const field = textFieldOf(p.target);
        if (!field) return;
        drag = { field, x: p.clientX, y: p.clientY, marked: false, lost: null };
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
        win.requestAnimationFrame(() => restoreFocus(d));
    };

    const onMove = (e: Event): void => {
        if (!drag) return;
        const p = e as PointerEvent;
        // The release happened where we could not see it, or the field was replaced under the drag.
        if (p.buttons === 0 || !drag.field.isConnected) {
            end();
            return;
        }
        if (!drag.marked && Math.hypot(p.clientX - drag.x, p.clientY - drag.y) >= DRAG_SLOP_PX) mark(drag);
    };

    const listeners: Array<[string, EventListener, boolean | undefined]> = [
        ["pointerdown", onDown, true],
        ["pointerup", end, true],
        ["pointercancel", end, true],
        ["pointermove", onMove, true],
        ["dragstart", onDragStart, true],
        ["focusout", onFocusOut, true],
        ["blur", end, undefined],
    ];
    for (const [type, fn, capture] of listeners) win.addEventListener(type, fn, capture);
    return () => {
        end();
        for (const [type, fn, capture] of listeners) win.removeEventListener(type, fn, capture);
    };
}
