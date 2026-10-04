// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { DEFAULT_KEYBINDINGS } from "./defaults";
import { formatKey, matchKey, parseKey, type KeyEventLike } from "./keys";
import { helpSections } from "./help";
import { _rowsForTests, commandForKey, findConflicts, formatCommand, matchPaneKey, resolveKey, setUserKeybindings, whenDisjoint, type KeyContext } from "./registry";

function ev(key: string, code: string, mods: Partial<KeyEventLike> = {}): KeyEventLike {
    return { key, code, ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...mods };
}

const NONE: KeyContext = { textInputFocus: false, terminalFocus: false, viewType: "term", docTabsHost: false };
const TERM: KeyContext = { textInputFocus: false, terminalFocus: true, viewType: "term", docTabsHost: false };
const TYPING: KeyContext = { textInputFocus: true, terminalFocus: false, viewType: "agent", docTabsHost: false };

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
        // Pane rows are exempt: their keys only reach a non-terminal pane.
        const bad = _rowsForTests("other").filter((c) => {
            if (c.row.pane) return false;
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

    it("resolves the pane swap / resize, tab move and rename keys", () => {
        expect(resolveKey(ev("ArrowLeft", "ArrowLeft", { ctrlKey: true, altKey: true, shiftKey: true }), NONE, "other")?.row.command).toBe("pane:swap:left");
        expect(resolveKey(ev("ArrowUp", "ArrowUp", { altKey: true, shiftKey: true }), TERM, "other")?.row.command).toBe("pane:resize:up");
        expect(resolveKey(ev("ArrowUp", "ArrowUp", { altKey: true, shiftKey: true }), TYPING, "other")).toBeNull();
        expect(resolveKey(ev("PageUp", "PageUp", { ctrlKey: true, altKey: true, shiftKey: true }), NONE, "other")?.row.command).toBe("tab:moveLeft");
        expect(resolveKey(ev("F2", "F2"), NONE, "other")?.row.command).toBe("tab:rename");
        expect(resolveKey(ev("F2", "F2"), TERM, "other")).toBeNull();
    });

    it("Ctrl+L focuses the message box only in an agent pane", () => {
        expect(resolveKey(ev("l", "KeyL", { ctrlKey: true }), { ...TYPING, viewType: "agent" }, "other")?.row.command).toBe("agent:focusComposer");
        expect(resolveKey(ev("l", "KeyL", { ctrlKey: true }), { ...NONE, viewType: "files" }, "other")).toBeNull();
    });

    it("labels commands for menus", () => {
        expect(formatCommand("tab:new", "other")).toBe("Ctrl+Shift+T");
        expect(formatCommand("tab:new", "mac")).toBe("⌘T");
        expect(formatCommand("view:command-palette", "other")).toBe("Ctrl+Shift+P");
        expect(formatCommand("pane:refocus", "other")).toBe("");
    });
});

describe("pane rows", () => {
    const FILES: KeyContext = { textInputFocus: false, terminalFocus: false, viewType: "files", docTabsHost: false };
    const EDITOR: KeyContext = { textInputFocus: true, terminalFocus: false, viewType: "editor", docTabsHost: true };

    it("a pane matches only its own rows", () => {
        expect(matchPaneKey(ev("N", "KeyN", { ctrlKey: true, shiftKey: true }), "files", "other")).toBe("files:newFolder");
        expect(matchPaneKey(ev("t", "KeyT", { ctrlKey: true }), "doctabs", "mac")).toBe("doctab:new");
        expect(matchPaneKey(ev("F", "KeyF", { ctrlKey: true, shiftKey: true }), "files", "other")).toBeNull();
    });

    it("the global dispatcher never runs a pane row; the global row is the fallback", () => {
        // In Files, the pane's own handler takes Ctrl+Shift+N first (new
        // folder) and marks it handled. When it doesn't (focus in its filter
        // box, say), the global row still applies.
        expect(resolveKey(ev("N", "KeyN", { ctrlKey: true, shiftKey: true }), FILES, "other")?.row.command).toBe("window:new");
        expect(resolveKey(ev("Tab", "Tab", { ctrlKey: true }), EDITOR, "other")?.row.command).toBe("tab:next");
        for (const c of _rowsForTests("other")) {
            if (c.row.pane) expect(resolveKey(ev("x", "KeyX"), NONE, "other")?.row.command).not.toBe(c.row.command);
        }
    });

    it("whenDisjoint understands flags, view types and docTabsHost", () => {
        expect(whenDisjoint("terminalFocus", "!terminalFocus")).toBe(true);
        expect(whenDisjoint("viewType == files", "viewType != files")).toBe(true);
        expect(whenDisjoint("viewType == files", "viewType == editor")).toBe(true);
        expect(whenDisjoint("docTabsHost", "viewType == files")).toBe(true);
        expect(whenDisjoint("!docTabsHost", "viewType == editor")).toBe(true);
        expect(whenDisjoint("docTabsHost", "viewType == editor")).toBe(false);
        expect(whenDisjoint("terminalFocus", "viewType == term")).toBe(false);
    });
});

describe("the keybindings setting", () => {
    afterEach(() => setUserKeybindings([]));
    const CSE = ev("E", "KeyE", { ctrlKey: true, shiftKey: true });
    const CST = ev("T", "KeyT", { ctrlKey: true, shiftKey: true });

    it("adds a key alongside the defaults", () => {
        expect(setUserKeybindings([{ key: "ctrl+shift+e", command: "split:right" }])).toEqual([]);
        expect(resolveKey(CSE, NONE, "other")?.row.command).toBe("split:right");
        expect(resolveKey(ev("D", "KeyD", { ctrlKey: true, shiftKey: true }), NONE, "other")?.row.command).toBe("split:right");
    });

    it("a user key wins over a default on the same key", () => {
        setUserKeybindings([{ key: "ctrl+shift+t", command: "pane:new" }]);
        expect(resolveKey(CST, NONE, "other")?.row.command).toBe("pane:new");
    });

    it("-command unbinds every key, or just the one given", () => {
        setUserKeybindings([{ command: "-tab:new" }]);
        expect(resolveKey(CST, NONE, "other")).toBeNull();
        expect(formatCommand("tab:new", "other")).toBe("");
        setUserKeybindings([{ command: "-tab:next", key: "ctrl+Tab" }]);
        expect(resolveKey(ev("Tab", "Tab", { ctrlKey: true }), NONE, "other")).toBeNull();
        expect(resolveKey(ev("}", "BracketRight", { ctrlKey: true, shiftKey: true }), NONE, "other")?.row.command).toBe("tab:next");
    });

    it("applies only to the platform it names", () => {
        setUserKeybindings([{ key: "ctrl+shift+e", command: "split:right", platform: "mac" }]);
        expect(resolveKey(CSE, NONE, "other")).toBeNull();
        expect(resolveKey(CSE, NONE, "mac")?.row.command).toBe("split:right");
    });

    it("skips a bad entry with a warning and keeps the rest", () => {
        const warnings = setUserKeybindings([
            { key: "ctrl+hyper+e", command: "split:right" },
            { key: "ctrl+shift+e", command: "split:right", when: "nosuchflag" },
            { key: "ctrl+shift+e" },
            { key: "ctrl+shift+e", command: "pane:new" },
        ]);
        expect(warnings).toHaveLength(3);
        expect(resolveKey(CSE, NONE, "other")?.row.command).toBe("pane:new");
    });

    it("rejects a chord of more than two keys, and an empty key", () => {
        const warnings = setUserKeybindings([
            { key: "ctrl+k ctrl+s ctrl+d", command: "pane:new" },
            { key: " ", command: "pane:new" },
            { key: "", command: "-tab:new" },
        ]);
        expect(warnings).toHaveLength(3);
        expect(resolveKey(ev("k", "KeyK", { ctrlKey: true }), NONE, "other")).toBeNull();
        expect(resolveKey(CST, NONE, "other")?.row.command).toBe("tab:new");
    });

    it("rejects an unknown flag anywhere in when, not only before the first false term", () => {
        expect(setUserKeybindings([{ key: "ctrl+shift+e", command: "pane:new", when: "terminalFocus && nosuchflag" }])).toHaveLength(1);
        expect(() => resolveKey(CSE, TERM, "other")).not.toThrow();
    });

    it("unbinds by key, whatever the spelling", () => {
        setUserKeybindings([{ command: "-tab:next", key: "Ctrl+Tab" }]);
        expect(resolveKey(ev("Tab", "Tab", { ctrlKey: true }), NONE, "other")).toBeNull();
        setUserKeybindings([{ command: "-tab:new", key: "mod+t" }]);
        expect(resolveKey(ev("t", "KeyT", { metaKey: true }), NONE, "mac")).toBeNull();
    });

    it("a forwarded browser-pane key ignores a user row that can't apply there", () => {
        setUserKeybindings([{ key: "ctrl+shift+t", command: "term:clear", when: "terminalFocus" }]);
        expect(commandForKey("ctrl+shift+t", "other")).toBe("tab:new");
        setUserKeybindings([{ key: "ctrl+shift+t", command: "pane:close" }]);
        expect(commandForKey("ctrl+shift+t", "other")).toBe("pane:close");
    });

    it("shows in the help pane", () => {
        setUserKeybindings([{ key: "ctrl+shift+e", command: "split:right" }]);
        const panes = helpSections("other").find((s) => s.category === "Panes");
        expect(panes?.entries.find((e) => e.label === "Split right")?.keys).toContain("Ctrl+Shift+E");
    });
});
