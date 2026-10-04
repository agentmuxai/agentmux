// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { DEFAULT_KEYBINDINGS } from "./defaults";
import { formatKey, matchKey, parseKey, type KeyEventLike } from "./keys";
import { _rowsForTests, findConflicts, formatCommand, resolveKey, type KeyContext } from "./registry";

function ev(key: string, code: string, mods: Partial<KeyEventLike> = {}): KeyEventLike {
    return { key, code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods };
}

const NONE: KeyContext = { textInputFocus: false, terminalFocus: false, viewType: "term" };
const TERM: KeyContext = { textInputFocus: false, terminalFocus: true, viewType: "term" };
const TYPING: KeyContext = { textInputFocus: true, terminalFocus: false, viewType: "agent" };

describe("key syntax", () => {
    it("formats per platform", () => {
        expect(formatKey("ctrl+shift+t", "other")).toBe("Ctrl+Shift+T");
        expect(formatKey("meta+shift+w", "mac")).toBe("⇧⌘W");
        expect(formatKey("mod+f", "mac")).toBe("⌘F");
        expect(formatKey("mod+f", "other")).toBe("Ctrl+F");
        expect(formatKey("ctrl+shift+code:Backquote", "other")).toBe("Ctrl+Shift+`");
        expect(formatKey("ctrl+shift+s ArrowUp", "other")).toBe("Ctrl+Shift+S then ↑");
        expect(formatKey("shift+F6", "mac")).toBe("⇧F6");
    });

    it("matches letters whatever the Shift state", () => {
        expect(matchKey(ev("T", "KeyT", { ctrlKey: true, shiftKey: true }), parseKey("ctrl+shift+t", "other"))).toBe(true);
    });

    it("matches digits by physical key (AZERTY types & on the 1 key)", () => {
        expect(matchKey(ev("&", "Digit1", { ctrlKey: true }), parseKey("ctrl+1", "other"))).toBe(true);
    });

    it("falls back to the physical key on a non-Latin layout", () => {
        expect(matchKey(ev("Е", "KeyT", { ctrlKey: true, shiftKey: true }), parseKey("ctrl+shift+t", "other"))).toBe(true);
    });

    it("doesn't take a character AltGr types", () => {
        const altGrAt = { ...ev("@", "KeyQ", { ctrlKey: true, altKey: true }), getModifierState: (k: string) => k === "AltGraph" };
        expect(matchKey(altGrAt, parseKey("ctrl+alt+q", "other"))).toBe(false);
    });

    it("requires the exact modifiers", () => {
        expect(matchKey(ev("t", "KeyT", { ctrlKey: true }), parseKey("ctrl+shift+t", "other"))).toBe(false);
    });
});

describe("default table", () => {
    it.each(["mac", "other"] as const)("has no conflicting bindings on %s", (platform) => {
        expect(findConflicts(platform)).toEqual([]);
    });

    it("never binds Alt+letter without Ctrl on Windows/Linux (the shell's Meta keys)", () => {
        const bad = _rowsForTests("other").filter((c) => {
            const k = c.steps[0];
            return k.alt && !k.ctrl && !k.meta && k.letter;
        });
        expect(bad.map((c) => c.source)).toEqual([]);
    });

    it("scopes every single-character binding (WCAG 2.1.4)", () => {
        for (const platform of ["mac", "other"] as const) {
            const bad = _rowsForTests(platform).filter((c) => {
                const k = c.steps[0];
                return !k.ctrl && !k.alt && !k.meta && (k.letter || k.code) && !c.row.when;
            });
            expect(bad.map((c) => c.source)).toEqual([]);
        }
    });

    it("every row has a key on at least one platform", () => {
        expect(DEFAULT_KEYBINDINGS.filter((r) => !(r.mac?.length || r.other?.length))).toEqual([]);
    });
});

describe("resolveKey", () => {
    it("gives the shell every key a binding doesn't explicitly take", () => {
        expect(resolveKey(ev("p", "KeyP", { ctrlKey: true }), TERM, "other")).toBeNull();
        expect(resolveKey(ev("w", "KeyW", { altKey: true }), TERM, "other")).toBeNull();
        expect(resolveKey(ev("f", "KeyF", { ctrlKey: true }), TERM, "other")).toBeNull();
        expect(resolveKey(ev("Escape", "Escape"), TERM, "other")).toBeNull();
    });

    it("runs the skip-list shortcuts in a terminal", () => {
        expect(resolveKey(ev("T", "KeyT", { ctrlKey: true, shiftKey: true }), TERM, "other")?.row.command).toBe("tab:new");
        expect(resolveKey(ev("F", "KeyF", { ctrlKey: true, shiftKey: true }), TERM, "other")?.row.command).toBe("pane:find");
        expect(resolveKey(ev("V", "KeyV", { ctrlKey: true, shiftKey: true }), TERM, "other")?.row.command).toBe("term:paste");
    });

    it("keeps Ctrl+P for the palette outside a terminal", () => {
        expect(resolveKey(ev("p", "KeyP", { ctrlKey: true }), NONE, "other")?.row.command).toBe("view:command-palette");
    });

    it("leaves word selection to a text field", () => {
        expect(resolveKey(ev("ArrowLeft", "ArrowLeft", { ctrlKey: true, shiftKey: true }), TYPING, "other")).toBeNull();
        expect(resolveKey(ev("ArrowLeft", "ArrowLeft", { ctrlKey: true, shiftKey: true }), NONE, "other")?.row.command).toBe("pane:focus:left");
    });

    it("Ctrl+Shift+V is voice outside a terminal", () => {
        expect(resolveKey(ev("V", "KeyV", { ctrlKey: true, shiftKey: true }), TYPING, "other")?.row.command).toBe("pane:voice");
    });

    it("resolves the split chord in two steps", () => {
        const first = resolveKey(ev("S", "KeyS", { ctrlKey: true, shiftKey: true }), NONE, "other");
        expect(first?.chordStart).toBe(true);
        expect(resolveKey(ev("ArrowUp", "ArrowUp"), NONE, "other", "ctrl+shift+s")?.row.command).toBe("split:up");
    });

    it("labels commands for menus", () => {
        expect(formatCommand("tab:new", "other")).toBe("Ctrl+Shift+T");
        expect(formatCommand("tab:new", "mac")).toBe("⌘T");
        expect(formatCommand("view:command-palette", "other")).toBe("Ctrl+Shift+P");
        expect(formatCommand("pane:refocus", "other")).toBe("");
    });
});
