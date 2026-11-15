// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One rule for every drag source: a press that starts on a control (a button, link, field, or anything marked
 * `data-no-drag`) inside the thing that would drag never starts that drag. A close button on a pane header, a
 * window tab, a pane tab or a document tab is the case that matters: the OS can lag between press and release,
 * a few pixels of movement count as a drag, and the pane or tab is torn off instead of closed.
 *
 * The press is read where it starts, not where the drag is first seen: the drag's own start event comes after the
 * pointer has already moved off the button.
 */

/** What a press on keeps for itself. A new button is covered with no change here. */
export const CONTROL_SELECTOR = "button, a, input, select, textarea, [role='button'], [data-no-drag]";
/** The pane grip is a `role="button"` that is a drag handle first: a press on it still drags. */
const HANDLE_SELECTOR = ".pane-grip, [data-drag-handle]";

/**
 * Whether `target` is, or is inside, a control. With `within`, only a control inside that element counts: the
 * draggable element is not a control of itself, and one around it is not either.
 */
export function pressOnControl(target: EventTarget | null, within?: Element): boolean {
    if (!(target instanceof Element)) return false;
    if (target.closest(HANDLE_SELECTOR)) return false;
    const control = target.closest(CONTROL_SELECTOR);
    if (!control) return false;
    return !within || (control !== within && within.contains(control));
}

/** What {@link guardControlPress} keeps for one drag source. */
export interface ControlPressGuard {
    /** The last press on the element began on a control. */
    pressedOnControl: () => boolean;
    /** Stops listening. */
    dispose: () => void;
}

/**
 * Remembers where each press on `el` began, for its drag source's `canDrag` (`!guard.pressedOnControl()`). The
 * listeners are capture-phase, so a control that stops its own press from bubbling is still seen.
 */
export function guardControlPress(el: HTMLElement): ControlPressGuard {
    let onControl = false;
    const press = (e: Event) => {
        onControl = pressOnControl(e.target, el);
    };
    el.addEventListener("pointerdown", press, true);
    el.addEventListener("mousedown", press, true);
    return {
        pressedOnControl: () => onControl,
        dispose: () => {
            el.removeEventListener("pointerdown", press, true);
            el.removeEventListener("mousedown", press, true);
        },
    };
}
