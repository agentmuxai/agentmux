// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The pure parts of the Linux L3 pass: the key events it sends through
// ydotool, and that it can't mistake unreadable GNOME keybindings for "none".

import { describe, expect, it } from "vitest";
import { readGnomeGrabs } from "./gnome-grabs.mjs";
import { keySequence } from "./verify-shortcuts-l3-linux.mjs";

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

describe("reading GNOME's keybindings", () => {
    it("says it couldn't read them, rather than returning an empty list, when gsettings is missing", () => {
        const path = process.env.PATH;
        process.env.PATH = "/nonexistent";
        try {
            const grabs = readGnomeGrabs();
            expect(grabs.readable).toBe(false);
            expect(grabs.size).toBe(0);
        } finally {
            process.env.PATH = path;
        }
    });
});
