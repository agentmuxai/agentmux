// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { DEFAULT_KEYBINDINGS } from "./defaults";
import { bindingsView, keybindingIssues, setUserKeybindings } from "./registry";

afterEach(() => setUserKeybindings([]));

const row = (command: string, platform: "mac" | "other" | "linux" = "other") => bindingsView(platform).find((r) => r.command === command)!;

describe("bindingsView", () => {
    it("lists every command once, in the table's order", () => {
        const commands = bindingsView("other").map((r) => r.command);
        expect(new Set(commands).size).toBe(commands.length);
        expect(commands).toEqual([...new Set(DEFAULT_KEYBINDINGS.map((r) => r.command))]);
    });

    it("gives a command's defaults and keys per platform, unchanged", () => {
        expect(row("tab:new", "mac")).toMatchObject({ defaults: ["meta+t"], keys: ["meta+t"], changed: false, unbound: false, label: "New tab", category: "Tabs & windows" });
        expect(row("tab:new")).toMatchObject({ defaults: ["ctrl+shift+t"], keys: ["ctrl+shift+t"], changed: false });
        expect(row("pane:swap:up", "linux").keys).toEqual(["ctrl+shift+s shift+ArrowUp"]);
        expect(row("pane:swap:up").keys).toEqual(["ctrl+alt+shift+ArrowUp"]);
    });

    it("keeps a command with several rows unchanged", () => {
        const find = row("pane:find");
        expect(find.changed).toBe(false);
        expect(find.keys).toEqual(["ctrl+f", "ctrl+shift+f"]);
        expect(find.keyRows.map((k) => k.when)).toEqual([undefined, "terminalFocus"]);
    });

    it("marks a rebound command as changed and keeps the default beside it", () => {
        setUserKeybindings([{ command: "-tab:new", key: "ctrl+shift+t", platform: "other" }, { command: "tab:new", key: "ctrl+alt+t", platform: "other" }]);
        expect(row("tab:new")).toMatchObject({ defaults: ["ctrl+shift+t"], keys: ["ctrl+alt+t"], changed: true, unbound: false });
        expect(row("tab:new").keyRows).toEqual([{ key: "ctrl+alt+t", when: undefined, user: true }]);
        expect(row("tab:new", "mac").changed).toBe(false);
    });

    it("treats a re-spelled default as unchanged", () => {
        setUserKeybindings([{ command: "-tab:new", key: "Ctrl+Shift+T" }, { command: "tab:new", key: "ctrl+shift+t" }]);
        expect(row("tab:new").changed).toBe(false);
    });

    it("shows an unbound command as Not set", () => {
        setUserKeybindings([{ command: "-tab:close" }]);
        expect(row("tab:close")).toMatchObject({ keys: [], unbound: true, changed: true });
    });

    it("counts a default key with a context of the person's own as a change", () => {
        setUserKeybindings([{ command: "-pane:close", key: "ctrl+shift+w" }, { command: "pane:close", key: "ctrl+shift+w", when: "!textInputFocus" }]);
        expect(row("pane:close").changed).toBe(true);
    });

    it("adds a command the table does not have only when the person bound it", () => {
        expect(bindingsView("other").some((r) => r.command === "open:files")).toBe(false);
        setUserKeybindings([{ command: "open:files", key: "ctrl+alt+f" }], (id) => id === "open:files");
        expect(bindingsView("other").at(-1)).toMatchObject({ command: "open:files", label: "open:files", category: "General", keys: ["ctrl+alt+f"], defaults: [], changed: true });
    });

    it("flags the dev-only rows", () => {
        expect(row("dev:perfHud").devOnly).toBe(true);
        expect(row("tab:new").devOnly).toBe(false);
    });
});

describe("skipped entries", () => {
    it("are listed with their place in the setting and the command they name", () => {
        const warnings = setUserKeybindings([
            { command: "tab:new", key: "ctrl+alt+t" },
            { command: "tab:ner", key: "ctrl+alt+n" },
            { command: "-nonsense" },
            { key: "ctrl+k" },
            { command: "tab:close", key: "a b c" },
        ]);
        expect(warnings).toHaveLength(4);
        expect(keybindingIssues().map((i) => [i.index, i.command])).toEqual([
            [1, "tab:ner"],
            [2, "nonsense"],
            [3, ""],
            [4, "tab:close"],
        ]);
        expect(row("tab:close").issues).toEqual(['"a b c": a chord is at most two keys']);
        expect(row("tab:new").issues).toEqual([]);
    });

    it("go away when the setting is fixed", () => {
        setUserKeybindings([{ command: "tab:ner", key: "ctrl+alt+n" }]);
        expect(keybindingIssues()).toHaveLength(1);
        setUserKeybindings([]);
        expect(keybindingIssues()).toEqual([]);
    });
});
