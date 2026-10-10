// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The pure parts of the Linux L3 pass: the key events it sends through
// ydotool, and how it undoes a workspace move GNOME made.

import { describe, expect, it } from "vitest";
import { eventFromNorm, keySequence, undoFor } from "./verify-shortcuts-l3-linux.mjs";

describe("keySequence", () => {
    it("presses the modifiers, then the key, and releases them in reverse", () => {
        // Ctrl+Shift+D: CDP modifiers Ctrl 2 | Shift 8.
        expect(keySequence({ code: "KeyD", modifiers: 10 })).toEqual(["29:1", "42:1", "32:1", "32:0", "42:0", "29:0"]);
        expect(keySequence({ code: "F1", modifiers: 0 })).toEqual(["59:1", "59:0"]);
        // Ctrl+Alt+Shift+ArrowUp, Alt 1.
        expect(keySequence({ code: "ArrowUp", modifiers: 11 })).toEqual(["29:1", "42:1", "56:1", "103:1", "103:0", "56:0", "42:0", "29:0"]);
    });

    it("refuses a key it has no Linux code for", () => {
        expect(() => keySequence({ code: "IntlRo", modifiers: 0 })).toThrow(/no Linux key code/);
    });
});

describe("undoing what GNOME did with a taken key", () => {
    const grabs = new Map([
        ["alt+ctrl+shift+arrowup", ["desktop.wm.keybindings move-to-workspace-up"]],
        ["alt+ctrl+shift+arrowdown", ["desktop.wm.keybindings move-to-workspace-down"]],
        ["alt+ctrl+arrowleft", ["desktop.wm.keybindings switch-to-workspace-left"]],
        ["alt+ctrl+arrowright", ["desktop.wm.keybindings switch-to-workspace-right"]],
        ["meta+a", ["shell.keybindings toggle-application-view"]],
    ]);

    it("undoes a workspace move or switch with the opposite binding", () => {
        expect(undoFor(["desktop.wm.keybindings move-to-workspace-up"], grabs)).toEqual({
            owner: "desktop.wm.keybindings move-to-workspace-up",
            back: "desktop.wm.keybindings move-to-workspace-down",
            ev: { code: "ArrowDown", modifiers: 11 },
        });
        expect(undoFor(["desktop.wm.keybindings switch-to-workspace-left"], grabs)?.ev).toEqual({ code: "ArrowRight", modifiers: 3 });
    });

    it("has no undo for anything else, or when the opposite isn't bound", () => {
        expect(undoFor(["shell.keybindings toggle-application-view"], grabs)).toBeNull();
        expect(undoFor(["desktop.wm.keybindings switch-to-workspace-up"], grabs)).toBeNull();
    });

    it("reads gnome-grabs' normal form back into a key event", () => {
        expect(eventFromNorm("alt+ctrl+arrowdown")).toEqual({ code: "ArrowDown", modifiers: 3 });
        expect(eventFromNorm("meta+a")).toBeNull();
    });
});
