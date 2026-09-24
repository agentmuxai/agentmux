// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SPEC_PANE_CLICK_THROUGH_INPUT_FOCUS_2026_09_23.md §4.1 — the one guard every
// pane-selection focus path consults before moving the caret.

import { afterEach, describe, expect, it } from "vitest";
import { eventBelongsToBlock, eventBelongsToPaneOf, userCaretInBlock } from "./focusutil";

function mountBlock(blockId: string, inner: string): HTMLElement {
    const block = document.createElement("div");
    block.setAttribute("data-blockid", blockId);
    block.innerHTML = inner;
    document.body.appendChild(block);
    return block;
}

describe("userCaretInBlock", () => {
    afterEach(() => {
        document.body.innerHTML = "";
    });

    it.each([
        ["input", `<input id="t" type="text" />`],
        ["textarea", `<textarea id="t"></textarea>`],
        ["select", `<select id="t"><option>a</option></select>`],
    ])("is true for a focused %s inside the block", (_name, html) => {
        mountBlock("A", html);
        document.getElementById("t")!.focus();
        expect(userCaretInBlock("A")).toBe(true);
    });

    it("is true for a focused contenteditable inside the block (CodeMirror)", () => {
        const block = mountBlock("A", `<div id="t" contenteditable="true" tabindex="0"></div>`);
        const el = block.querySelector<HTMLElement>("#t")!;
        // jsdom doesn't compute isContentEditable from the attribute.
        Object.defineProperty(el, "isContentEditable", { value: true });
        el.focus();
        expect(userCaretInBlock("A")).toBe(true);
    });

    it("is false for the block's own dummy-focus input", () => {
        mountBlock("A", `<input id="A-dummy-focus" class="dummy-focus" />`);
        document.getElementById("A-dummy-focus")!.focus();
        expect(userCaretInBlock("A")).toBe(false);
    });

    it("is false when the caret is in a DIFFERENT block", () => {
        mountBlock("A", `<input id="a" />`);
        mountBlock("B", `<input id="b" />`);
        document.getElementById("b")!.focus();
        expect(userCaretInBlock("A")).toBe(false);
    });

    it("is false for a focused button — #3519 still routes the caret to the pane's input", () => {
        mountBlock("A", `<button id="t">go</button>`);
        document.getElementById("t")!.focus();
        expect(userCaretInBlock("A")).toBe(false);
    });

    it("is false for a non-text input type (checkbox)", () => {
        mountBlock("A", `<input id="t" type="checkbox" />`);
        document.getElementById("t")!.focus();
        expect(userCaretInBlock("A")).toBe(false);
    });

    it("is false when nothing is focused", () => {
        mountBlock("A", `<input id="t" />`);
        expect(userCaretInBlock("A")).toBe(false);
    });
});

describe("eventBelongsToPaneOf", () => {
    afterEach(() => {
        document.body.innerHTML = "";
    });

    const keydownFrom = (target: EventTarget): KeyboardEvent => {
        const e = new KeyboardEvent("keydown", { key: "Escape", bubbles: true });
        Object.defineProperty(e, "target", { value: target });
        return e;
    };

    it("is true for an event from inside the same pane", () => {
        const a = mountBlock("A", `<div id="overlay"></div><textarea id="composer"></textarea>`);
        expect(eventBelongsToPaneOf(keydownFrom(a.querySelector("#composer")!), a.querySelector("#overlay"))).toBe(true);
    });

    it("is false for an event from ANOTHER pane — the two-panes-side-by-side leak", () => {
        const a = mountBlock("A", `<div id="overlay"></div>`);
        const b = mountBlock("B", `<textarea id="composer"></textarea>`);
        expect(eventBelongsToPaneOf(keydownFrom(b.querySelector("#composer")!), a.querySelector("#overlay"))).toBe(
            false
        );
    });

    it("attributes a pane-less target (body) to the SELECTED pane only", () => {
        const a = mountBlock("A", `<div id="overlay"></div>`);
        const b = mountBlock("B", `<div id="overlay-b"></div>`);
        b.classList.add("block-focused");
        expect(eventBelongsToPaneOf(keydownFrom(document.body), a.querySelector("#overlay"))).toBe(false);
        expect(eventBelongsToPaneOf(keydownFrom(document.body), b.querySelector("#overlay-b"))).toBe(true);
    });

    it("treats an element outside any pane as global (always true)", () => {
        const loose = document.createElement("div");
        document.body.appendChild(loose);
        const b = mountBlock("B", `<textarea id="composer"></textarea>`);
        expect(eventBelongsToPaneOf(keydownFrom(b.querySelector("#composer")!), loose)).toBe(true);
    });
});

describe("eventBelongsToBlock", () => {
    afterEach(() => {
        document.body.innerHTML = "";
        document.getSelection()?.removeAllRanges();
    });

    const keydownFrom = (target: EventTarget): KeyboardEvent => {
        const e = new KeyboardEvent("keydown", { key: "f", ctrlKey: true, bubbles: true });
        Object.defineProperty(e, "target", { value: target });
        return e;
    };

    it("belongs to the pane the key came from", () => {
        mountBlock("A", `<textarea id="a"></textarea>`);
        const b = mountBlock("B", `<textarea id="b"></textarea>`);
        const e = keydownFrom(b.querySelector("#b")!);
        expect(eventBelongsToBlock(e, "B")).toBe(true);
        expect(eventBelongsToBlock(e, "A")).toBe(false);
    });

    it("ignores a text selection left in ANOTHER pane — the caret's pane wins (Ctrl+F bug)", () => {
        const a = mountBlock("A", `<p id="txt">selected transcript text</p>`);
        const b = mountBlock("B", `<textarea id="b"></textarea>`);
        const range = document.createRange();
        range.selectNodeContents(a.querySelector("#txt")!);
        document.getSelection()!.addRange(range);

        const e = keydownFrom(b.querySelector("#b")!);
        expect(eventBelongsToBlock(e, "A")).toBe(false);
        expect(eventBelongsToBlock(e, "B")).toBe(true);
    });

    it("a key from outside every pane (body) belongs to the SELECTED pane — including via its chrome's block id", () => {
        mountBlock("A", ``);
        const b = mountBlock("B", ``);
        b.classList.add("block-focused");
        const e = keydownFrom(document.body);
        expect(eventBelongsToBlock(e, "A")).toBe(false);
        expect(eventBelongsToBlock(e, "B")).toBe(true);
    });
});
