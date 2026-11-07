// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { setPlatform } from "@/util/platformutil";
import { createMemo, createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    focusSearchField,
    registerSearchControls,
    restoreFocusToMagnifier,
    searchControlsFor,
    searchHeaderButtons,
} from "./search-bridge";

beforeEach(() => setPlatform("win32"));
afterEach(() => {
    document.body.innerHTML = "";
    setPlatform("win32");
});

function pane(visible: () => boolean = () => false) {
    const open = vi.fn();
    const toggle = vi.fn();
    const off = registerSearchControls("blk", { visible, open, toggle });
    return { open, toggle, off };
}

describe("the find bar's bridge", () => {
    it("gives the header a magnifier while the pane has registered its bar, and takes it back", () => {
        expect(searchHeaderButtons("blk")).toEqual([]);
        const { off } = pane();
        const [button] = searchHeaderButtons("blk");
        expect(button).toMatchObject({
            elemtype: "toggleiconbutton",
            icon: "magnifying-glass",
            className: "agent-search-toggle",
        });
        expect(button.title).toBe("Find in conversation (Ctrl+F)");
        off();
        expect(searchHeaderButtons("blk")).toEqual([]);
        expect(searchControlsFor("blk")).toBeUndefined();
    });

    it("names Cmd+F on macOS", () => {
        setPlatform("darwin");
        const { off } = pane();
        expect(searchHeaderButtons("blk")[0].title).toBe("Find in conversation (Cmd+F)");
        off();
    });

    it("is pressed while the bar is open, and a click toggles it through the pane", () => {
        const [open, setOpen] = createSignal(false);
        const { toggle, off } = pane(open);
        const [button] = searchHeaderButtons("blk");
        expect(button.active()).toBe(false);
        setOpen(true);
        expect(button.active()).toBe(true);
        button.active._set(false);
        expect(toggle).toHaveBeenCalledOnce();
        off();
    });

    it("keeps the same button across reads, so the header does not redraw it", () => {
        const { off } = pane();
        expect(searchHeaderButtons("blk")[0]).toBe(searchHeaderButtons("blk")[0]);
        off();
    });

    it("an old registration cannot take away a newer one for the same pane", () => {
        const first = pane();
        const second = pane();
        first.off();
        expect(searchHeaderButtons("blk")).toHaveLength(1);
        second.off();
        expect(searchHeaderButtons("blk")).toHaveLength(0);
    });

    it("is reactive: a header reading the buttons sees the pane arrive and leave", () => {
        createRoot((dispose) => {
            const count = createMemo(() => searchHeaderButtons("blk").length);
            expect(count()).toBe(0);
            const { off } = pane();
            expect(count()).toBe(1);
            off();
            expect(count()).toBe(0);
            dispose();
        });
    });
});

describe("focus", () => {
    function frame(): { field: HTMLInputElement; toggle: HTMLButtonElement } {
        document.body.innerHTML = `<div data-blockid="blk"><button class="agent-search-toggle"></button><div class="agent-search-bar"><input class="agent-search-input" value="hello"></div></div><input id="elsewhere">`;
        return {
            field: document.querySelector(".agent-search-input") as HTMLInputElement,
            toggle: document.querySelector(".agent-search-toggle") as HTMLButtonElement,
        };
    }

    it("puts the caret in the field with its text selected", () => {
        const { field } = frame();
        focusSearchField("blk");
        expect(document.activeElement).toBe(field);
        expect(field.selectionStart).toBe(0);
        expect(field.selectionEnd).toBe(5);
    });

    it("returns focus to the magnifier when focus was in the bar, or nowhere", () => {
        const { field, toggle } = frame();
        field.focus();
        restoreFocusToMagnifier("blk");
        expect(document.activeElement).toBe(toggle);
        (document.activeElement as HTMLElement).blur();
        restoreFocusToMagnifier("blk");
        expect(document.activeElement).toBe(toggle);
    });

    it("leaves focus alone when it is somewhere else on the page", () => {
        frame();
        const elsewhere = document.getElementById("elsewhere") as HTMLInputElement;
        elsewhere.focus();
        restoreFocusToMagnifier("blk");
        expect(document.activeElement).toBe(elsewhere);
    });
});
