// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { installTextDragGuard } from "./text-drag";

let uninstall: () => void;

function press(target: Element, init: { button?: number; pointerType?: string } = {}): void {
    const ev = new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: init.button ?? 0 });
    Object.assign(ev, { pointerType: init.pointerType ?? "mouse" });
    target.dispatchEvent(ev);
}

function pointer(type: string, buttons: number, target: Element = document.body): void {
    const ev = new MouseEvent(type, { bubbles: true, cancelable: true, buttons });
    target.dispatchEvent(ev);
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
    it("marks the page and the field while a press in a text field is held, and clears both on release", () => {
        const root = mount('<input id="f" value="hello world">');
        const f = root.querySelector("input")!;
        press(f);
        expect(dragging()).toBe(true);
        expect(f.hasAttribute("data-text-drag-field")).toBe(true);
        pointer("pointerup", 0, f);
        expect(dragging()).toBe(false);
        expect(f.hasAttribute("data-text-drag-field")).toBe(false);
    });

    it.each(["text", "search", "email", "url", "tel", "password", "number"])("guards a %s field and a textarea", (type) => {
        const root = mount(`<input type="${type}"><textarea></textarea>`);
        press(root.querySelector("input")!);
        expect(dragging()).toBe(true);
        pointer("pointercancel", 0);
        press(root.querySelector("textarea")!);
        expect(dragging()).toBe(true);
    });

    it("leaves alone a press that isn't a mouse's primary button in a text field", () => {
        const root = mount('<input type="checkbox"><input type="range"><button>b</button><div id="d">x</div><input id="t">');
        for (const el of root.querySelectorAll("input[type=checkbox], input[type=range], button, #d")) {
            press(el);
            expect(dragging()).toBe(false);
        }
        press(root.querySelector("#t")!, { button: 2 });
        expect(dragging()).toBe(false);
        press(root.querySelector("#t")!, { pointerType: "touch" });
        expect(dragging()).toBe(false);
    });

    it("leaves the terminal's own input and a marked field alone", () => {
        const root = mount('<div class="xterm"><textarea id="x"></textarea></div><div data-text-drag-skip><input id="m"></div>');
        press(root.querySelector("#x")!);
        expect(dragging()).toBe(false);
        press(root.querySelector("#m")!);
        expect(dragging()).toBe(false);
    });

    it("stops a drag an ancestor of the field would start, so the press keeps selecting", () => {
        const root = mount('<div id="pane" draggable="true"><input id="f"></div>');
        const pane = root.querySelector("#pane")!;
        const f = root.querySelector("#f")!;
        const paneDrag = vi.fn();
        pane.addEventListener("dragstart", paneDrag);

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
        press(f);
        pointer("pointermove", 1);
        expect(dragging()).toBe(true);
        pointer("pointermove", 0);
        expect(dragging()).toBe(false);

        press(f);
        window.dispatchEvent(new Event("blur"));
        expect(dragging()).toBe(false);
    });

    it("ends when the field is replaced during the drag", () => {
        const root = mount("<input>");
        press(root.querySelector("input")!);
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
        press(f);
        expect(dragging()).toBe(false);
        uninstall = installTextDragGuard(window);
    });
});
