// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What the person's own keys do to the places that show a key: a change reaches Help, the palette hint and a
// tooltip without a reload, Help names the default beside a key that was replaced, and a command left with no
// key stays listed (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §6).

import { createComputed, createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/util/platformutil", () => ({ isMacOS: () => false, isLinux: () => false }));

import { listShortcuts } from "./app-api";
import { helpSections } from "./help";
import { codeMirrorKeys, shortcutFor, shortcutHelp } from "./index";
import { setUserKeybindings } from "./registry";

afterEach(() => setUserKeybindings([]));

const entry = (command: string) =>
    helpSections("other")
        .flatMap((s) => s.entries)
        .find((e) => e.command === command)!;

describe("a rebound key reaches whatever shows it", () => {
    it("re-runs a reader of shortcutFor, shortcutHelp and codeMirrorKeys", () => {
        const tip: string[] = [];
        const help: string[] = [];
        const editor: string[][] = [];
        const dispose = createRoot((d) => {
            createComputed(() => tip.push(shortcutFor("tab:new")));
            createComputed(() => help.push(shortcutHelp().flatMap((s) => s.entries).find((e) => e.command === "tab:new")!.keys.join()));
            createComputed(() => editor.push(codeMirrorKeys("editor:save")));
            return d;
        });
        setUserKeybindings([
            { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
            { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
            { command: "-editor:save", key: "ctrl+s", platform: "other" },
            { command: "editor:save", key: "ctrl+alt+s", platform: "other" },
        ]);
        dispose();
        expect(tip).toEqual(["Ctrl+Shift+T", "Ctrl+Alt+T"]);
        expect(help).toEqual(["Ctrl+Shift+T", "Ctrl+Alt+T"]);
        expect(editor).toEqual([["Ctrl-s"], ["Ctrl-Alt-s"]]);
    });
});

describe("Help", () => {
    it("shows no default note for a key nobody changed", () => {
        expect(entry("tab:new")).toMatchObject({ keys: ["Ctrl+Shift+T"], command: "tab:new" });
        expect(entry("tab:new").defaults).toBeUndefined();
        expect(entry("tab:new").unbound).toBeUndefined();
    });

    it("names the default beside a key that was replaced", () => {
        setUserKeybindings([
            { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
            { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
        ]);
        expect(entry("tab:new")).toMatchObject({ keys: ["Ctrl+Alt+T"], defaults: ["Ctrl+Shift+T"] });
    });

    it("lists a command with no key as Not set, with its default", () => {
        setUserKeybindings([{ command: "-tab:close", platform: "other" }]);
        expect(entry("tab:close")).toMatchObject({ keys: [], unbound: true, defaults: ["Ctrl+Alt+Shift+W", "Ctrl+F4"] });
    });

    it("keeps a grouped line while any of its commands has a key", () => {
        setUserKeybindings([{ command: "-tab:goto:3", platform: "other" }]);
        const group = helpSections("other").flatMap((s) => s.entries).filter((e) => e.label === "Go to tab 1–8");
        expect(group).toHaveLength(1);
        expect(group[0].unbound).toBeUndefined();
    });
});

describe("ListShortcuts", () => {
    it("tells the person's key from the default", () => {
        setUserKeybindings([
            { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
            { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
        ]);
        const info = listShortcuts("other").find((s) => s.command === "tab:new")!;
        expect(info).toMatchObject({ raw: ["ctrl+alt+t"], defaults: ["ctrl+shift+t"], changed: true, unbound: false });
        expect(listShortcuts("other").find((s) => s.command === "tab:close")).toMatchObject({ changed: false, unbound: false });
    });

    it("still lists a command the person left with no key, so it runs by name", () => {
        setUserKeybindings([{ command: "-tab:close", platform: "other" }]);
        expect(listShortcuts("other").find((s) => s.command === "tab:close")).toMatchObject({ keys: [], raw: [], unbound: true, changed: true });
    });

    it("does not list a command that never had a key on this platform", () => {
        expect(listShortcuts("other").some((s) => s.command === "pane:refocus")).toBe(false);
    });
});
