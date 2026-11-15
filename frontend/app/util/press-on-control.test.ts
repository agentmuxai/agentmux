// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { CONTROL_SELECTOR, guardControlPress, pressOnControl } from "./press-on-control";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, parent?: Element, attrs: Record<string, string> = {}) {
    const node = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
    parent?.appendChild(node);
    return node;
}

afterEach(() => document.body.replaceChildren());

describe("pressOnControl", () => {
    it("is true on a button, and on anything inside one", () => {
        const button = el("button", document.body);
        const icon = el("span", button);
        expect(pressOnControl(button)).toBe(true);
        expect(pressOnControl(icon)).toBe(true);
    });

    it("covers a link, a field, a role=button and a data-no-drag element", () => {
        for (const node of [
            el("a", document.body, { href: "#x" }),
            el("input", document.body),
            el("select", document.body),
            el("textarea", document.body),
            el("div", document.body, { role: "button" }),
            el("div", document.body, { "data-no-drag": "" }),
        ]) {
            expect(pressOnControl(node), node.outerHTML).toBe(true);
        }
        expect(CONTROL_SELECTOR).toContain("button");
    });

    it("is false on a header or tab body", () => {
        const header = el("div", document.body, { "data-role": "block-header" });
        const label = el("span", header);
        expect(pressOnControl(header)).toBe(false);
        expect(pressOnControl(label)).toBe(false);
        expect(pressOnControl(null)).toBe(false);
    });

    it("leaves the pane grip a drag handle although it is a role=button", () => {
        const grip = el("div", document.body, { role: "button", class: "pane-grip" });
        const dots = el("span", grip);
        expect(pressOnControl(grip)).toBe(false);
        expect(pressOnControl(dots)).toBe(false);
    });

    it("with `within`, counts only a control inside that element", () => {
        const outer = el("button", document.body);
        const tab = el("div", outer);
        const inside = el("span", tab);
        const close = el("button", tab);
        // The draggable is not a control of itself, nor is a control around it.
        expect(pressOnControl(inside, tab)).toBe(false);
        expect(pressOnControl(close, tab)).toBe(true);
        const selfButton = el("button", document.body);
        expect(pressOnControl(selfButton, selfButton)).toBe(false);
    });
});

describe("guardControlPress", () => {
    it("remembers where the last press began, whatever the pointer does after", () => {
        const tab = el("div", document.body);
        const close = el("button", tab);
        const body = el("span", tab);
        const guard = guardControlPress(tab);
        expect(guard.pressedOnControl()).toBe(false);

        close.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
        expect(guard.pressedOnControl()).toBe(true);
        // A move away and the drag's start do not change it.
        body.dispatchEvent(new MouseEvent("pointermove", { bubbles: true }));
        expect(guard.pressedOnControl()).toBe(true);

        body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
        expect(guard.pressedOnControl()).toBe(false);
        guard.dispose();
    });

    it("sees a press that the control stops from bubbling", () => {
        const tab = el("div", document.body);
        const close = el("button", tab);
        close.addEventListener("mousedown", (e) => e.stopPropagation());
        const guard = guardControlPress(tab);
        close.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
        expect(guard.pressedOnControl()).toBe(true);
        guard.dispose();
    });

    it("stops listening when disposed", () => {
        const tab = el("div", document.body);
        const close = el("button", tab);
        const guard = guardControlPress(tab);
        guard.dispose();
        close.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
        expect(guard.pressedOnControl()).toBe(false);
    });
});
