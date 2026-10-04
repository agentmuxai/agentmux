// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The dispatcher leaves a key alone when something closer to the user
// already handled it, and keeps text-editing keys for the field being typed
// in (docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md §3).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    atoms: { modalOpen: () => false },
    getApi: () => ({ setKeyboardChordMode: vi.fn(), onControlShiftStateUpdate: vi.fn() }),
    getBlockComponentModel: () => null,
    setControlShiftDelayAtom: vi.fn(),
}));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({ focusedNode: () => null }),
}));

import { appHandleKeyDown, globalKeyMap, isTypingFocus } from "./keymodel-dispatch";
import { adaptFromReactOrNativeKeyEvent, setKeyUtilPlatform } from "@/util/keyutil";

function press(init: KeyboardEventInit, prevent = false): boolean {
    const ev = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
    if (prevent) ev.preventDefault();
    return appHandleKeyDown(adaptFromReactOrNativeKeyEvent(ev));
}

describe("appHandleKeyDown", () => {
    const paneFocus = vi.fn(() => true);
    const newTab = vi.fn(() => true);
    let field: HTMLElement | null = null;

    beforeEach(() => {
        setKeyUtilPlatform("win32");
        globalKeyMap.clear();
        globalKeyMap.set("Ctrl:Shift:ArrowLeft", paneFocus);
        globalKeyMap.set("Cmd:t", newTab);
        paneFocus.mockClear();
        newTab.mockClear();
    });

    afterEach(() => {
        field?.remove();
        field = null;
        globalKeyMap.clear();
    });

    function focus(el: HTMLElement): void {
        field = el;
        document.body.appendChild(el);
        el.focus();
    }

    it("runs a global shortcut normally", () => {
        expect(press({ key: "ArrowLeft", ctrlKey: true, shiftKey: true })).toBe(true);
        expect(paneFocus).toHaveBeenCalledTimes(1);
    });

    it("skips a key a pane or the editor already handled", () => {
        expect(press({ key: "t", altKey: true }, true)).toBe(false);
        expect(newTab).not.toHaveBeenCalled();
    });

    it("leaves word selection to a focused text field", () => {
        focus(document.createElement("textarea"));
        expect(press({ key: "ArrowLeft", ctrlKey: true, shiftKey: true })).toBe(false);
        expect(paneFocus).not.toHaveBeenCalled();
    });

    it("still runs other global shortcuts while typing", () => {
        focus(document.createElement("textarea"));
        expect(press({ key: "t", altKey: true })).toBe(true);
        expect(newTab).toHaveBeenCalledTimes(1);
    });

    it("doesn't treat the terminal's hidden textarea as typing", () => {
        const ta = document.createElement("textarea");
        ta.className = "xterm-helper-textarea";
        focus(ta);
        expect(isTypingFocus()).toBe(false);
        expect(press({ key: "ArrowLeft", ctrlKey: true, shiftKey: true })).toBe(true);
    });
});
