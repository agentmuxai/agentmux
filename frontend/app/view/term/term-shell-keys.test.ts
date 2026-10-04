// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { terminalKeyGoesToShell, type ShellKeyEvent } from "./term-shell-keys";

function key(code: string, mods: Partial<ShellKeyEvent> = {}): ShellKeyEvent {
    return { code, ctrlKey: false, altKey: false, metaKey: false, shiftKey: false, ...mods };
}

describe("terminalKeyGoesToShell", () => {
    it("gives readline's Alt+letter keys to the shell on Windows/Linux", () => {
        for (const code of ["KeyW", "KeyD", "KeyT", "KeyF", "KeyB"]) {
            expect(terminalKeyGoesToShell(key(code, { altKey: true }), false)).toBe(true);
        }
        expect(terminalKeyGoesToShell(key("KeyW", { altKey: true, shiftKey: true }), false)).toBe(true);
    });

    it("leaves Alt+digit and Alt+brackets to the app (tab switching)", () => {
        expect(terminalKeyGoesToShell(key("Digit1", { altKey: true }), false)).toBe(false);
        expect(terminalKeyGoesToShell(key("BracketRight", { altKey: true }), false)).toBe(false);
    });

    it("leaves AltGr (Ctrl+Alt on Windows) alone", () => {
        expect(terminalKeyGoesToShell(key("KeyQ", { altKey: true, ctrlKey: true }), false)).toBe(false);
    });

    it("does not take Option+letter on macOS, where app shortcuts use ⌘", () => {
        expect(terminalKeyGoesToShell(key("KeyW", { altKey: true }), true)).toBe(false);
    });

    it("gives Ctrl+P, Ctrl+[ and Ctrl+] to the shell on every platform", () => {
        for (const isMac of [false, true]) {
            expect(terminalKeyGoesToShell(key("KeyP", { ctrlKey: true }), isMac)).toBe(true);
            expect(terminalKeyGoesToShell(key("BracketLeft", { ctrlKey: true }), isMac)).toBe(true);
            expect(terminalKeyGoesToShell(key("BracketRight", { ctrlKey: true }), isMac)).toBe(true);
        }
    });

    it("leaves Ctrl+Shift shortcuts to the app", () => {
        expect(terminalKeyGoesToShell(key("KeyP", { ctrlKey: true, shiftKey: true }), false)).toBe(false);
        expect(terminalKeyGoesToShell(key("ArrowLeft", { ctrlKey: true, shiftKey: true }), false)).toBe(false);
    });
});
