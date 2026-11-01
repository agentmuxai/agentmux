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
 * So while the primary button is held after a press in a text field, this guard
 *
 *  - stops a `dragstart` that comes from an ancestor of the field (the field's own text drag, moving a
 *    selection, is left alone);
 *  - marks the field `data-text-drag-field` and the root `data-text-drag`, which the kit stylesheet turns
 *    into `pointer-events: none` for everything else: nothing is hovered, nothing receives the release as
 *    a click, and so nothing reacts;
 *  - puts the focus and the selection back if something took the focus off the field anyway.
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

interface Drag {
    field: HTMLInputElement | HTMLTextAreaElement;
    /** Where the selection was when the focus left, to put back. */
    lost: { start: number; end: number; dir: "forward" | "backward" | "none" } | null;
}

export function installTextDragGuard(win: Window = window): () => void {
    const doc = win.document;
    let drag: Drag | null = null;

    const end = (): void => {
        if (!drag) return;
        drag.field.removeAttribute(FIELD_ATTR);
        doc.documentElement.removeAttribute(ROOT_ATTR);
        drag = null;
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
        drag = { field, lost: null };
        field.setAttribute(FIELD_ATTR, "");
        doc.documentElement.setAttribute(ROOT_ATTR, "");
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
        // The release happened where we could not see it, or the field was replaced under the drag.
        if ((e as PointerEvent).buttons === 0 || !drag.field.isConnected) end();
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
