// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The dispatcher runs the shortcut table (keybindings/defaults.ts) against
// where focus is: a key something closer already handled is left alone, a
// text field keeps its editing keys, and a terminal keeps every key that
// isn't on the skip-list (docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md §3, §11.3).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const listeners = vi.hoisted(() => new Map<string, (payload: unknown) => void>());
vi.mock("@/app/store/global", () => ({
    atoms: { modalOpen: () => false },
    getApi: () => ({
        listen: (event: string, cb: (payload: unknown) => void) => {
            listeners.set(event, cb);
            return Promise.resolve(() => {});
        },
        reclaimWindowFocus: () => Promise.resolve(),
    }),
    getBlockComponentModel: () => null,
    setControlShiftDelayAtom: vi.fn(),
}));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({ focusedNode: () => null }),
}));
vi.mock("@/app/store/command-registry", () => ({ commandRegistry: { run: vi.fn(() => false) } }));

import { appHandleKeyDown, disableGlobalKeybindings, enableGlobalKeybindings, isTypingFocus, keyCommands, registerHostShortcuts } from "./keymodel-dispatch";
import { adaptFromReactOrNativeKeyEvent, setKeyUtilPlatform } from "@/util/keyutil";
import { setPlatform } from "@/util/platformutil";
import { setUserKeybindings } from "@/app/keybindings/registry";

function press(init: KeyboardEventInit, prevent = false): boolean {
    const ev = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
    if (prevent) ev.preventDefault();
    return appHandleKeyDown(adaptFromReactOrNativeKeyEvent(ev));
}

const CTRL_SHIFT_LEFT = { key: "ArrowLeft", code: "ArrowLeft", ctrlKey: true, shiftKey: true };
const CTRL_SHIFT_T = { key: "T", code: "KeyT", ctrlKey: true, shiftKey: true };

describe("appHandleKeyDown", () => {
    const handlers = {
        "pane:focus:left": vi.fn(() => true),
        "tab:new": vi.fn(() => true),
        "pane:close": vi.fn(() => true),
        "view:command-palette": vi.fn(() => true),
        "split:up": vi.fn(() => true),
    };
    let field: HTMLElement | null = null;

    beforeEach(() => {
        setKeyUtilPlatform("win32");
        setPlatform("win32");
        keyCommands.clear();
        for (const [command, fn] of Object.entries(handlers)) {
            fn.mockClear();
            keyCommands.set(command, fn);
        }
    });

    afterEach(() => {
        field?.remove();
        field = null;
        keyCommands.clear();
        setPlatform("darwin");
        setKeyUtilPlatform("darwin");
    });

    function focus(el: HTMLElement): void {
        field = el;
        document.body.appendChild(el);
        el.focus();
    }

    function focusTerminal(): void {
        const ta = document.createElement("textarea");
        ta.className = "xterm-helper-textarea";
        focus(ta);
    }

    it("runs a shortcut from the table", () => {
        expect(press(CTRL_SHIFT_LEFT)).toBe(true);
        expect(handlers["pane:focus:left"]).toHaveBeenCalledTimes(1);
    });

    it("skips a key a pane or the editor already handled", () => {
        expect(press(CTRL_SHIFT_T, true)).toBe(false);
        expect(handlers["tab:new"]).not.toHaveBeenCalled();
    });

    it("leaves word selection to a focused text field", () => {
        focus(document.createElement("textarea"));
        expect(press(CTRL_SHIFT_LEFT)).toBe(false);
        expect(handlers["pane:focus:left"]).not.toHaveBeenCalled();
    });

    it("still runs other shortcuts while typing", () => {
        focus(document.createElement("textarea"));
        expect(press(CTRL_SHIFT_T)).toBe(true);
        expect(handlers["tab:new"]).toHaveBeenCalledTimes(1);
    });

    it("doesn't treat the terminal's hidden textarea as typing", () => {
        focusTerminal();
        expect(isTypingFocus()).toBe(false);
        expect(press(CTRL_SHIFT_LEFT)).toBe(true);
    });

    it("leaves the shell's keys to a focused terminal", () => {
        focusTerminal();
        expect(press({ key: "w", code: "KeyW", altKey: true })).toBe(false);
        expect(press({ key: "p", code: "KeyP", ctrlKey: true })).toBe(false);
        expect(handlers["view:command-palette"]).not.toHaveBeenCalled();
        expect(press({ key: "W", code: "KeyW", ctrlKey: true, shiftKey: true })).toBe(true);
        expect(handlers["pane:close"]).toHaveBeenCalledTimes(1);
    });

    it("runs the split chord's second key", () => {
        expect(press({ key: "S", code: "KeyS", ctrlKey: true, shiftKey: true })).toBe(true);
        expect(press({ key: "ArrowUp", code: "ArrowUp" })).toBe(true);
        expect(handlers["split:up"]).toHaveBeenCalledTimes(1);
    });

    it("runs a shortcut the host forwards out of a browser pane, after taking focus back", async () => {
        registerHostShortcuts();
        listeners.get("app-shortcut")?.({ block_id: "b1", command: "tab:new" });
        await vi.waitFor(() => expect(handlers["tab:new"]).toHaveBeenCalledTimes(1));
        disableGlobalKeybindings();
        listeners.get("app-shortcut")?.({ block_id: "b1", command: "tab:new" });
        enableGlobalKeybindings();
        await new Promise((r) => setTimeout(r, 0));
        expect(handlers["tab:new"]).toHaveBeenCalledTimes(1);
    });

    it("skips a forwarded key the user's keybindings unbound", async () => {
        registerHostShortcuts();
        setUserKeybindings([{ command: "-tab:new", key: "ctrl+shift+t" }]);
        listeners.get("app-shortcut")?.({ block_id: "b1", command: "tab:new", key: "ctrl+shift+t" });
        await new Promise((r) => setTimeout(r, 0));
        expect(handlers["tab:new"]).not.toHaveBeenCalled();
        setUserKeybindings([]);
        listeners.get("app-shortcut")?.({ block_id: "b1", command: "tab:new", key: "ctrl+shift+t" });
        await vi.waitFor(() => expect(handlers["tab:new"]).toHaveBeenCalledTimes(1));
    });
});
