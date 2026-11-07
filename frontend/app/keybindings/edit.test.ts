// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { choicesOf, conflictsFor, planChange } from "./edit";
import { setUserKeybindings } from "./registry";

afterEach(() => setUserKeybindings([]));

const keysOf = (entries: unknown[], command: string, platform: "mac" | "other" | "linux") => choicesOf(entries, command, platform).map((c) => c.key);

describe("planChange: the smallest list of entries", () => {
    it("writes nothing when the keys are the defaults", () => {
        expect(planChange([], "tab:new", [{ key: "ctrl+shift+t" }], "other").entries).toEqual([]);
    });

    it("adds a key as one plain entry pinned to the platform", () => {
        const { entries } = planChange([], "tab:new", [{ key: "ctrl+shift+t" }, { key: "ctrl+alt+t" }], "other");
        expect(entries).toEqual([{ command: "tab:new", key: "ctrl+alt+t", platform: "other" }]);
        expect(keysOf(entries, "tab:new", "other")).toEqual(["ctrl+alt+t", "ctrl+shift+t"]);
    });

    it("replaces a default: unbind it, then add the new one", () => {
        const { entries } = planChange([], "tab:new", [{ key: "ctrl+alt+t" }], "other");
        expect(entries).toEqual([
            { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
            { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
        ]);
        expect(keysOf(entries, "tab:new", "other")).toEqual(["ctrl+alt+t"]);
    });

    it("writes one bare unbind when every key is dropped, so a later default stays off", () => {
        const { entries } = planChange([], "tab:close", [], "other");
        expect(entries).toEqual([{ command: "-tab:close", platform: "other" }]);
        expect(keysOf(entries, "tab:close", "other")).toEqual([]);
        expect(keysOf(entries, "tab:close", "mac")).toEqual(["meta+shift+w"]);
    });

    it("removes one of several default keys with its own unbind", () => {
        const { entries } = planChange([], "tab:close", [{ key: "ctrl+shift+alt+w" }], "other");
        expect(entries).toEqual([{ command: "-tab:close", key: "ctrl+F4", platform: "other" }]);
    });

    it("resets a command by wanting its defaults back, leaving other commands alone", () => {
        const before = planChange([], "tab:new", [{ key: "ctrl+alt+t" }], "other").entries;
        const both = planChange(before, "pane:close", [{ key: "ctrl+alt+w" }], "other").entries;
        const after = planChange(both, "tab:new", [{ key: "ctrl+shift+t" }], "other").entries;
        expect(after).toEqual([
            { command: "-pane:close", key: "ctrl+shift+w", platform: "other" },
            { command: "pane:close", key: "ctrl+alt+w", platform: "other" },
        ]);
    });

    it("keeps the other platform's entries, and keeps an unpinned entry for the platform not being edited", () => {
        const start = [
            { command: "tab:new", key: "ctrl+alt+t" },
            { command: "tab:new", key: "meta+alt+t", platform: "mac" },
        ];
        const { entries } = planChange(start, "tab:new", [{ key: "meta+t" }], "mac");
        expect(entries).toEqual([
            { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
            // The Mac's own entry is the one being replaced.
        ]);
        expect(keysOf(entries, "tab:new", "mac")).toEqual(["meta+t"]);
        expect(keysOf(entries, "tab:new", "other")).toEqual(["ctrl+alt+t", "ctrl+shift+t"]);
    });

    it("keeps entries it cannot read and puts a command's group where its first entry was", () => {
        const start = [{ command: "nope", key: "ctrl+k" }, null, { command: "tab:new", key: "ctrl+alt+t", platform: "other" }, { command: "split:right", key: "ctrl+alt+r", platform: "other" }];
        const { entries } = planChange(start, "tab:new", [{ key: "ctrl+alt+y" }, { key: "ctrl+shift+t" }], "other");
        expect(entries).toEqual([{ command: "nope", key: "ctrl+k" }, null, { command: "tab:new", key: "ctrl+alt+y", platform: "other" }, { command: "split:right", key: "ctrl+alt+r", platform: "other" }]);
    });

    it("writes a context for a key that needs one", () => {
        const { entries } = planChange([], "pane:close", [{ key: "ctrl+shift+w" }, { key: "ctrl+alt+w", when: "!textInputFocus" }], "other");
        expect(entries).toEqual([{ command: "pane:close", key: "ctrl+alt+w", when: "!textInputFocus", platform: "other" }]);
    });

    it("tells a default's key and its own context apart", () => {
        // pane:find has the terminal-only Ctrl+Shift+F: the same key without that context is a new entry.
        const { entries } = planChange([], "pane:find", [{ key: "ctrl+f" }, { key: "ctrl+shift+f" }], "other");
        expect(entries).toEqual([
            { command: "-pane:find", key: "ctrl+shift+f", platform: "other" },
            { command: "pane:find", key: "ctrl+shift+f", platform: "other" },
        ]);
    });

    it("pins to other on Linux, and uses Linux's own default column", () => {
        // The default's context comes with its key: a key with no `when` is a different binding.
        const swap = { key: "ctrl+shift+s shift+ArrowUp", when: "!textInputFocus" };
        expect(planChange([], "pane:swap:up", [{ key: swap.key }], "linux").entries).toHaveLength(2);
        expect(planChange([], "pane:swap:up", [swap], "linux").entries).toEqual([]);
        expect(planChange([], "pane:swap:up", [swap], "other").entries).toEqual([
            { command: "-pane:swap:up", key: "ctrl+alt+shift+ArrowUp", platform: "other" },
            { command: "pane:swap:up", key: swap.key, when: swap.when, platform: "other" },
        ]);
    });

    it("writes entries the registry accepts", () => {
        const { entries } = planChange([], "tab:new", [{ key: "mod+alt+t" }, { key: "ctrl+shift+e   ArrowDown" }], "other");
        expect(setUserKeybindings(entries)).toEqual([]);
        expect(keysOf(entries, "tab:new", "other")).toContain("ctrl+shift+e ArrowDown");
    });

    it("does not touch a command it was not asked about when a default is added later", () => {
        const { entries } = planChange([], "tab:new", [{ key: "ctrl+alt+t" }], "other");
        expect(entries.every((e) => (e as { command: string }).command.replace(/^-/, "") === "tab:new")).toBe(true);
    });
});

describe("conflictsFor", () => {
    it("finds the command that has the key in the same places", () => {
        const found = conflictsFor({ command: "pane:close", key: "ctrl+shift+d" }, [], "other");
        expect(found).toEqual([expect.objectContaining({ command: "split:right", kind: "same", blocking: true })]);
    });

    it("does not count the command's own key", () => {
        expect(conflictsFor({ command: "split:right", key: "ctrl+shift+d" }, [], "other")).toEqual([]);
    });

    it("sees a single key that hides a chord, and a chord behind a single key", () => {
        expect(conflictsFor({ command: "tab:new", key: "ctrl+shift+s" }, [], "other")).toEqual(
            expect.arrayContaining([expect.objectContaining({ kind: "shadows", blocking: true })])
        );
        expect(conflictsFor({ command: "tab:new", key: "ctrl+shift+d ArrowUp" }, [], "other")).toEqual([
            expect.objectContaining({ command: "split:right", kind: "shadowed", blocking: true }),
        ]);
    });

    it("allows two chords that share a first step", () => {
        // The editor's own Ctrl+Shift+S is a pane's key, so it is only noted.
        const found = conflictsFor({ command: "tab:new", key: "ctrl+shift+s ArrowUp" }, [], "other");
        expect(found.filter((c) => c.blocking)).toEqual([]);
    });

    it("ignores a row whose context cannot hold at the same time", () => {
        const found = conflictsFor({ command: "pane:close", key: "F2", when: "textInputFocus" }, [], "other");
        expect(found.map((c) => c.command)).not.toContain("tab:rename");
    });

    it("notes a pane's own key without blocking on it", () => {
        const found = conflictsFor({ command: "tab:close", key: "ctrl+shift+n" }, [], "other");
        expect(found.find((c) => c.command === "window:new")).toMatchObject({ kind: "same", blocking: true });
        expect(found.find((c) => c.command === "files:newFolder")).toMatchObject({ kind: "pane", blocking: false });
    });

    it("reads the person's own keys too", () => {
        const mine = [{ command: "tab:new", key: "ctrl+alt+t", platform: "other" }];
        expect(conflictsFor({ command: "pane:close", key: "ctrl+alt+t" }, mine, "other")).toEqual([expect.objectContaining({ command: "tab:new" })]);
        expect(conflictsFor({ command: "pane:close", key: "ctrl+alt+t" }, mine, "mac")).toEqual([]);
    });

    it("sees a key that is gone because the person unbound it", () => {
        const mine = [{ command: "-split:right", key: "ctrl+shift+d", platform: "other" }];
        expect(conflictsFor({ command: "pane:close", key: "ctrl+shift+d" }, mine, "other")).toEqual([]);
    });
});

describe("planChange with conflicts", () => {
    it("reports what the new keys collide with, and does not change the other command", () => {
        const plan = planChange([], "pane:close", [{ key: "ctrl+shift+d" }], "other");
        expect(plan.conflicts).toEqual([expect.objectContaining({ command: "split:right", blocking: true })]);
        expect(keysOf(plan.entries, "split:right", "other")).toContain("ctrl+shift+d");
    });

    it("replaces: unbinds the other command's key and leaves its other keys", () => {
        const plan = planChange([], "pane:close", [{ key: "ctrl+shift+d" }], "other", { replace: true });
        expect(plan.conflicts.filter((c) => c.blocking)).toEqual([]);
        expect(keysOf(plan.entries, "pane:close", "other")).toEqual(["ctrl+shift+d"]);
        expect(keysOf(plan.entries, "split:right", "other")).toEqual(["ctrl+shift+s ArrowRight"]);
        expect(setUserKeybindings(plan.entries)).toEqual([]);
    });

    it("never replaces a pane's own key", () => {
        const plan = planChange([], "tab:close", [{ key: "ctrl+shift+n" }], "other", { replace: true });
        expect(keysOf(plan.entries, "files:newFolder", "other")).toEqual(["ctrl+shift+n"]);
        expect(plan.conflicts.every((c) => !c.blocking)).toBe(true);
    });

    it("reports a reserved key that was added", () => {
        const plan = planChange([], "tab:new", [{ key: "Escape" }, { key: "ctrl+alt+t" }, { key: "ctrl+shift+e" }], "other");
        expect(plan.reserved).toEqual([
            { key: "Escape", reserved: expect.objectContaining({ tier: "blocked" }) },
            { key: "ctrl+alt+t", reserved: expect.objectContaining({ tier: "warned" }) },
        ]);
    });
});
