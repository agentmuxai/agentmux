// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DRAG_SLOP_PX, installTextDragGuard } from "./text-drag";

let uninstall: () => void;

function press(target: Element, init: { button?: number; pointerType?: string } = {}): void {
    const ev = new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: init.button ?? 0, clientX: 100, clientY: 100 });
    Object.assign(ev, { pointerType: init.pointerType ?? "mouse" });
    target.dispatchEvent(ev);
}

/** A pointer event at `dx` px from the press (a drag when past the slop, a click's jitter when under it). */
function pointer(type: string, buttons: number, target: Element = document.body, dx = 40): void {
    const ev = new MouseEvent(type, { bubbles: true, cancelable: true, buttons, clientX: 100 + dx, clientY: 100 });
    target.dispatchEvent(ev);
}

/** A press that has become a drag: the pointer moved past the slop with the button held. */
function drag(target: Element): void {
    press(target);
    pointer("pointermove", 1, target);
}

const dragging = () => document.documentElement.hasAttribute("data-text-drag");

beforeEach(() => {
    document.body.innerHTML = "";
    uninstall = installTextDragGuard(window);
});

afterEach(() => {
    uninstall();
    vi.useRealTimers();
});

function mount(html: string): HTMLElement {
    const root = document.createElement("div");
    root.innerHTML = html;
    document.body.append(root);
    return root;
}

describe("the text drag guard", () => {
    it("marks the page and the field once a press in a text field moves, and clears both on release", () => {
        const root = mount('<input id="f" value="hello world">');
        const f = root.querySelector("input")!;
        drag(f);
        expect(dragging()).toBe(true);
        expect(f.hasAttribute("data-text-drag-field")).toBe(true);
        pointer("pointerup", 0, f);
        expect(dragging()).toBe(false);
        expect(f.hasAttribute("data-text-drag-field")).toBe(false);
    });

    it("marks nothing for a click: a press and release that does not move, or moves less than the slop", () => {
        // `html[data-text-drag] *` restyles the whole document, so a click into a composer must not toggle it (#45977).
        const root = mount('<input id="f" value="hello world">');
        const f = root.querySelector("input")!;
        const html = document.documentElement;
        const seen: string[] = [];
        const mo = new MutationObserver((rs) => rs.forEach((r) => seen.push(String(r.attributeName))));
        mo.observe(html, { attributes: true });
        mo.observe(f, { attributes: true });

        press(f);
        expect(dragging()).toBe(false);
        pointer("pointermove", 1, f, DRAG_SLOP_PX - 1);
        expect(dragging()).toBe(false);
        pointer("pointerup", 0, f, DRAG_SLOP_PX - 1);
        expect(dragging()).toBe(false);
        mo.disconnect();
        expect(seen).toEqual([]);

        press(f);
        pointer("pointermove", 1, f, DRAG_SLOP_PX);
        expect(dragging()).toBe(true);
    });

    it.each(["text", "search", "email", "url", "tel", "password", "number"])("guards a %s field and a textarea", (type) => {
        const root = mount(`<input type="${type}"><textarea></textarea>`);
        drag(root.querySelector("input")!);
        expect(dragging()).toBe(true);
        pointer("pointercancel", 0);
        drag(root.querySelector("textarea")!);
        expect(dragging()).toBe(true);
    });

    it("leaves alone a press that isn't a mouse's primary button in a text field", () => {
        const root = mount('<input type="checkbox"><input type="range"><button>b</button><div id="d">x</div><input id="t">');
        for (const el of root.querySelectorAll("input[type=checkbox], input[type=range], button, #d")) {
            drag(el);
            expect(dragging()).toBe(false);
        }
        press(root.querySelector("#t")!, { button: 2 });
        pointer("pointermove", 2);
        expect(dragging()).toBe(false);
        press(root.querySelector("#t")!, { pointerType: "touch" });
        pointer("pointermove", 1);
        expect(dragging()).toBe(false);
    });

    it("leaves the terminal's own input and a marked field alone", () => {
        const root = mount('<div class="xterm"><textarea id="x"></textarea></div><div data-text-drag-skip><input id="m"></div>');
        drag(root.querySelector("#x")!);
        expect(dragging()).toBe(false);
        drag(root.querySelector("#m")!);
        expect(dragging()).toBe(false);
    });

    it("stops a drag an ancestor of the field would start, so the press keeps selecting", () => {
        const root = mount('<div id="pane" draggable="true"><input id="f"></div>');
        const pane = root.querySelector("#pane")!;
        const f = root.querySelector("#f")!;
        const paneDrag = vi.fn();
        pane.addEventListener("dragstart", paneDrag);

        // From the press alone: a `dragstart` comes before any move the guard sees.
        press(f);
        const ev = new Event("dragstart", { bubbles: true, cancelable: true });
        pane.dispatchEvent(ev);
        expect(paneDrag).not.toHaveBeenCalled();
        expect(ev.defaultPrevented).toBe(true);
    });

    it("lets the field's own text drag, and every drag when no press is held, through", () => {
        const root = mount('<div id="pane" draggable="true"><input id="f"></div>');
        const pane = root.querySelector("#pane")!;
        const f = root.querySelector("#f")!;
        const paneDrag = vi.fn();
        pane.addEventListener("dragstart", paneDrag);

        pane.dispatchEvent(new Event("dragstart", { bubbles: true, cancelable: true }));
        expect(paneDrag).toHaveBeenCalledTimes(1);

        press(f);
        const own = new Event("dragstart", { bubbles: true, cancelable: true });
        f.dispatchEvent(own);
        expect(own.defaultPrevented).toBe(false);
        expect(paneDrag).toHaveBeenCalledTimes(2);
    });

    it("ends when the pointer moves with no button down, and the window loses focus", () => {
        const f = mount("<input>").querySelector("input")!;
        drag(f);
        expect(dragging()).toBe(true);
        pointer("pointermove", 0);
        expect(dragging()).toBe(false);

        drag(f);
        expect(dragging()).toBe(true);
        window.dispatchEvent(new Event("blur"));
        expect(dragging()).toBe(false);
    });

    it("ends when the field is replaced during the drag", () => {
        const root = mount("<input>");
        drag(root.querySelector("input")!);
        expect(dragging()).toBe(true);
        root.innerHTML = "<input>";
        pointer("pointermove", 1);
        expect(dragging()).toBe(false);
    });

    it("puts the focus and the selection back when something takes the focus off the field", () => {
        vi.useFakeTimers({ toFake: ["requestAnimationFrame"] });
        const root = mount('<input id="f" value="hello world"><input id="other">');
        const f = root.querySelector<HTMLInputElement>("#f")!;
        const other = root.querySelector<HTMLInputElement>("#other")!;
        f.focus();
        // From the press alone, before any move: the theft can happen at once.
        press(f);
        // The drag so far: the whole value, from the right end to the left.
        f.setSelectionRange(0, 11, "backward");

        other.focus();
        expect(document.activeElement).toBe(other);
        vi.advanceTimersByTime(20);

        expect(document.activeElement).toBe(f);
        expect([f.selectionStart, f.selectionEnd, f.selectionDirection]).toEqual([0, 11, "backward"]);
    });

    it("does not pull the focus back after the release", () => {
        vi.useFakeTimers({ toFake: ["requestAnimationFrame"] });
        const root = mount('<input id="f" value="hello"><input id="other">');
        const f = root.querySelector<HTMLInputElement>("#f")!;
        const other = root.querySelector<HTMLInputElement>("#other")!;
        f.focus();
        press(f);
        other.focus();
        pointer("pointerup", 0, other);
        vi.advanceTimersByTime(20);
        expect(document.activeElement).toBe(other);
    });

    it("stops listening when uninstalled", () => {
        const f = mount("<input>").querySelector("input")!;
        uninstall();
        drag(f);
        expect(dragging()).toBe(false);
        uninstall = installTextDragGuard(window);
    });
});
