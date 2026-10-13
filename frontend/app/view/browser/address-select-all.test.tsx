// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createFirstClickSelectAll } from "./address-select-all";

const URL = "https://example.com/";

describe("createFirstClickSelectAll", () => {
    let input: HTMLInputElement;
    let other: HTMLButtonElement;
    let h: ReturnType<typeof createFirstClickSelectAll>;

    beforeEach(() => {
        input = document.createElement("input");
        input.value = URL;
        other = document.createElement("button");
        document.body.append(input, other);
        vi.spyOn(document, "hasFocus").mockReturnValue(true);
        h = createFirstClickSelectAll();
        input.addEventListener("focus", () => h.onFocus(input));
    });
    afterEach(() => {
        input.remove();
        other.remove();
        vi.restoreAllMocks();
    });

    /** A click: mousedown, focus moves in (if it isn't there), the browser
     *  places the caret (or a drag selection), mouseup. */
    const click = (caret: number, dragTo = caret) => {
        h.onMouseDown(input);
        input.focus();
        input.setSelectionRange(caret, dragTo);
        h.onMouseUp(input);
        document.dispatchEvent(new MouseEvent("mouseup"));
    };
    const selection = () => [input.selectionStart, input.selectionEnd];

    it("selects the whole address on the click that focuses the bar", () => {
        other.focus();
        click(8);
        expect(selection()).toEqual([0, URL.length]);
    });

    it("places the caret on a click while the bar already has focus", () => {
        input.focus();
        click(8);
        expect(selection()).toEqual([8, 8]);
    });

    it("keeps a drag selection made by the focusing click", () => {
        other.focus();
        click(8, 15);
        expect(selection()).toEqual([8, 15]);
    });

    it("selects all when the bar had focus but its window didn't (a click from the page)", () => {
        input.focus();
        vi.spyOn(document, "hasFocus").mockReturnValue(false);
        click(8);
        expect(selection()).toEqual([0, URL.length]);
    });

    it("selects only once: the next click places the caret", () => {
        other.focus();
        click(8);
        click(3);
        expect(selection()).toEqual([3, 3]);
    });

    it("selects all at once when focus comes from the keyboard", () => {
        other.focus();
        input.focus();
        expect(selection()).toEqual([0, URL.length]);
    });

    it("counts a press as over when its mouseup lands outside the bar", () => {
        other.focus();
        h.onMouseDown(input);
        input.focus();
        input.setSelectionRange(8, 15);
        // The drag ends outside: only the document sees the mouseup.
        document.dispatchEvent(new MouseEvent("mouseup"));
        input.blur();
        input.focus();
        expect(selection()).toEqual([0, URL.length]);
    });
});
