// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The wheel over the hover panel (the panel a row shows: a tool's command and output). A panel with something to
 * scroll keeps the wheel to itself: the transcript behind it does not move, so the row stays under the pointer and the
 * panel stays open. Moving the transcript is what closed it (the row left the pointer), after the panel had scrolled.
 *
 * "Something to scroll" is the panel itself, or a part inside it (the log, a wide command), in the wheel's own
 * direction. At the end of that part the wheel does nothing: it never chains on, and a trackpad's momentum that
 * carries past the end is held the same way, event by event. A panel with nothing to scroll in that direction is
 * left to scroll-handoff.ts, which hands the wheel to the transcript.
 */

export type Axis = "x" | "y";

/** What the panel does with one wheel event it keeps. */
export type PanelWheel =
    /** The browser scrolls the part under the pointer, which has room. */
    | { kind: "native" }
    /** Nothing in the panel can move that way: the event is cancelled and nothing moves. */
    | { kind: "hold" }
    /** A part other than the one under the pointer takes it, by hand: the browser would not (the part under the
     *  pointer is at its end, or the pointer is on a part that does not scroll, such as the header). */
    | { kind: "move"; el: HTMLElement; axis: Axis; by: number };

/** The room a fractional layout leaves between a true edge and the browser's own figure. */
const SLACK = 1;
/** A wheel of lines is this many pixels a line. */
const LINE_PX = 40;

const roomOf = (el: HTMLElement, axis: Axis): number =>
    axis === "y" ? el.scrollHeight - el.clientHeight : el.scrollWidth - el.clientWidth;

/** The part scrolls on this axis: it has more than fits, and its overflow lets the wheel move it. */
function scrolls(el: HTMLElement, axis: Axis): boolean {
    if (roomOf(el, axis) <= SLACK) return false;
    const style = getComputedStyle(el);
    const mode = axis === "y" ? style.overflowY : style.overflowX;
    return mode === "auto" || mode === "scroll" || mode === "overlay";
}

/** The part can move one way from where it is. */
function canMove(el: HTMLElement, axis: Axis, dir: -1 | 1): boolean {
    const at = axis === "y" ? el.scrollTop : el.scrollLeft;
    return dir < 0 ? at > 0 : at + SLACK < roomOf(el, axis);
}

/** The wheel's axis and signed distance. Shift turns a vertical wheel sideways, as the browser does. */
function wheelOf(e: Pick<WheelEvent, "deltaX" | "deltaY" | "shiftKey">): { axis: Axis; delta: number } | null {
    const { deltaX: x, deltaY: y } = e;
    if (e.shiftKey && x === 0 && y !== 0) return { axis: "x", delta: y };
    if (Math.abs(x) > Math.abs(y)) return { axis: "x", delta: x };
    return y === 0 ? null : { axis: "y", delta: y };
}

/** The parts that scroll on `axis`, from the one under the pointer outwards to the panel itself. */
function scrollersUnder(root: HTMLElement, target: Element | null, axis: Axis): HTMLElement[] {
    const found: HTMLElement[] = [];
    for (let n = target; n && root.contains(n); n = n.parentElement) {
        if (n instanceof HTMLElement && scrolls(n, axis)) found.push(n);
    }
    return found;
}

/** The first part inside the panel that scrolls on `axis`, for a pointer on a part that does not (the header). */
function scrollerWithin(root: HTMLElement, axis: Axis): HTMLElement | null {
    for (const n of root.querySelectorAll<HTMLElement>("*")) if (scrolls(n, axis)) return n;
    return null;
}

function pixels(e: Pick<WheelEvent, "deltaMode">, delta: number, el: HTMLElement, axis: Axis): number {
    if (e.deltaMode === 1) return delta * LINE_PX;
    if (e.deltaMode === 2) return delta * (axis === "y" ? el.clientHeight : el.clientWidth);
    return delta;
}

/**
 * One wheel event over the panel: what the panel does with it, or null when nothing in it scrolls the wheel's way and
 * the hand-off to the transcript decides.
 */
export function planPanelWheel(
    root: HTMLElement,
    target: Element | null,
    e: Pick<WheelEvent, "deltaX" | "deltaY" | "deltaMode" | "shiftKey">,
): PanelWheel | null {
    const wheel = wheelOf(e);
    if (!wheel) return null;
    const dir = wheel.delta < 0 ? -1 : 1;
    const under = scrollersUnder(root, target, wheel.axis);
    const within = under.length > 0 ? null : scrollerWithin(root, wheel.axis);
    const candidates = within ? [within] : under;
    if (candidates.length === 0) return null;
    const mover = candidates.find((el) => canMove(el, wheel.axis, dir));
    if (!mover) return { kind: "hold" };
    if (mover === under[0]) return { kind: "native" };
    return { kind: "move", el: mover, axis: wheel.axis, by: pixels(e, wheel.delta, mover, wheel.axis) };
}
