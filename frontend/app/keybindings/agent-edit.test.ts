// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";

import { planAgentReset, planAgentSet } from "./agent-edit";
import { setUserKeybindings } from "./registry";

const NEW_TAB_MINE = [
    { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
    { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
];

beforeEach(() => void setUserKeybindings([], () => false));

describe("planAgentSet", () => {
    it("gives a command the keys, written as a diff from the defaults", () => {
        const r = planAgentSet([], { command: "tab:new", bindings: ["ctrl+shift+y"] }, "other");
        expect(r).toEqual({
            changed: true,
            count: 1,
            keys: ["ctrl+shift+y"],
            summary: "New tab: Ctrl+Shift+Y",
            entries: [
                { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
                { command: "tab:new", key: "ctrl+shift+y", platform: "other" },
            ],
        });
    });

    it("unbinds a command with an empty list", () => {
        const r = planAgentSet([], { command: "tab:close", bindings: [] }, "other");
        expect(r).toMatchObject({ changed: true, keys: [], summary: "Close tab: no key", entries: [{ command: "-tab:close", platform: "other" }] });
    });

    it("works where the command works today, and where it is told", () => {
        const own = planAgentSet([], { command: "term:copy", bindings: ["ctrl+shift+y"] }, "other");
        expect(own).toMatchObject({ changed: true });
        expect((own as { entries: unknown[] }).entries).toContainEqual({ command: "term:copy", key: "ctrl+shift+y", when: "terminalFocus && viewType == term", platform: "other" });
        const told = planAgentSet([], { command: "tab:new", bindings: ["ctrl+shift+y"], when: "!terminalFocus" }, "other");
        expect((told as { entries: unknown[] }).entries).toContainEqual({ command: "tab:new", key: "ctrl+shift+y", when: "!terminalFocus", platform: "other" });
    });

    it("takes a blank place as no place given: the key works where the command works today", () => {
        // `null`: srv sends an absent `when` as null.
        for (const when of ["", "   ", null as unknown as string]) {
            const plan = planAgentSet([], { command: "term:copy", bindings: ["ctrl+shift+y"], when }, "other");
            expect((plan as { entries: unknown[] }).entries).toContainEqual({ command: "term:copy", key: "ctrl+shift+y", when: "terminalFocus && viewType == term", platform: "other" });
        }
    });

    it("leaves a key the person types with to the person: it would type into every text field and terminal", () => {
        for (const key of ["a", "5", "shift+a", "code:Space", "a ctrl+k"]) {
            const plan = planAgentSet([], { command: "tab:new", bindings: [key] }, "other");
            expect(plan, key).toMatchObject({ changed: false });
            expect((plan as { reason: string }).reason, key).toContain("Ask the person to set it in Settings");
        }
        // Function keys, arrows and keys with Ctrl, Alt or Meta are not typing.
        for (const key of ["F9", "ctrl+alt+9", "alt+shift+a"]) {
            expect(planAgentSet([], { command: "tab:new", bindings: [key] }, "other"), key).toMatchObject({ changed: true });
        }
    });

    it("takes cmd and command as the Meta key", () => {
        for (const key of ["cmd+shift+y", "command+shift+y"]) {
            expect(planAgentSet([], { command: "tab:new", bindings: [key] }, "mac"), key).toMatchObject({ changed: true });
        }
    });

    it("says so when nothing would change", () => {
        expect(planAgentSet([], { command: "tab:new", bindings: ["ctrl+shift+t"] }, "other")).toEqual({ changed: false, reason: "New tab already has Ctrl+Shift+T" });
        expect(planAgentSet(NEW_TAB_MINE, { command: "tab:new", bindings: ["ctrl+alt+t"] }, "other")).toMatchObject({ changed: false });
    });

    it("refuses a command that is not in the table", () => {
        expect(planAgentSet([], { command: "tab:nope", bindings: ["ctrl+y"] }, "other")).toMatchObject({ changed: false, reason: expect.stringMatching(/^unknown command "tab:nope"/) });
        expect(planAgentSet([], { command: "", bindings: [] }, "other")).toMatchObject({ changed: false });
    });

    it("refuses a key that does not parse, a chord of three, and bindings that are not a list", () => {
        expect(planAgentSet([], { command: "tab:new", bindings: ["ctrl+nonsense+x"] }, "other")).toMatchObject({ changed: false, reason: expect.stringMatching(/unknown modifier/) });
        expect(planAgentSet([], { command: "tab:new", bindings: ["ctrl+a ctrl+b ctrl+c"] }, "other")).toMatchObject({ changed: false, reason: expect.stringMatching(/at most two keys/) });
        expect(planAgentSet([], { command: "tab:new", bindings: [" "] }, "other")).toMatchObject({ changed: false, reason: "a key is empty" });
        expect(planAgentSet([], { command: "tab:new", bindings: "ctrl+y" as unknown as string[] }, "other")).toMatchObject({ changed: false, reason: expect.stringMatching(/must be a list/) });
    });

    it("refuses a place a shortcut cannot work", () => {
        expect(planAgentSet([], { command: "tab:new", bindings: ["ctrl+shift+y"], when: "no such flag" }, "other")).toMatchObject({ changed: false, reason: expect.stringMatching(/not a place a shortcut can work/) });
    });

    it("refuses a key the system takes, and sends a key the desktop often takes to the person", () => {
        expect(planAgentSet([], { command: "tab:new", bindings: ["alt+F4"] }, "other")).toMatchObject({ changed: false, reason: expect.stringMatching(/^Alt\+F4 cannot be used: Windows takes Alt\+F4/) });
        expect(planAgentSet([], { command: "tab:new", bindings: ["ctrl+alt+t"] }, "other")).toMatchObject({ changed: false, reason: expect.stringMatching(/Ask the person to set it in Settings/) });
    });

    it("never takes a key from another command: it returns what has it", () => {
        const r = planAgentSet([], { command: "tab:new", bindings: ["ctrl+shift+w"] }, "other");
        expect(r).toEqual({
            changed: false,
            reason: "Ctrl+Shift+W is Close pane; an agent does not take a key from another command (ask the person, or use another key)",
            taken: [{ command: "pane:close", label: "Close pane", key: "ctrl+shift+w" }],
        });
    });

    it("lets a key sit beside a command that works somewhere else", () => {
        // Copy has ctrl+shift+c in a terminal only.
        expect(planAgentSet([], { command: "tab:new", bindings: ["ctrl+shift+c"], when: "!terminalFocus" }, "other")).toMatchObject({ changed: true });
    });
});

describe("planAgentReset", () => {
    it("puts a command back on its defaults", () => {
        const r = planAgentReset(NEW_TAB_MINE, { command: "tab:new" }, "other");
        expect(r).toEqual({ changed: true, count: 1, entries: [], keys: ["ctrl+shift+t"], summary: "New tab back on its default: Ctrl+Shift+T" });
    });

    it("leaves the other commands' entries as they are", () => {
        const other = { command: "-pane:close", platform: "other" };
        expect(planAgentReset([...NEW_TAB_MINE, other], { command: "tab:new" }, "other")).toMatchObject({ changed: true, entries: [other] });
    });

    it("says so when the command is already on its defaults, or is not in the table", () => {
        expect(planAgentReset([], { command: "tab:new" }, "other")).toEqual({ changed: false, reason: "New tab is already on its defaults" });
        expect(planAgentReset([], { command: "tab:nope" }, "other")).toMatchObject({ changed: false, reason: expect.stringMatching(/^unknown command "tab:nope"/) });
        expect(planAgentReset([], {}, "other")).toMatchObject({ changed: false });
    });

    it("resets every command, counting the ones that were changed", () => {
        const entries = [...NEW_TAB_MINE, { command: "-pane:close", platform: "other" }];
        setUserKeybindings(entries, () => false);
        expect(planAgentReset(entries, { all: true }, "other")).toEqual({
            changed: true,
            count: 2,
            entries: [],
            keys: [],
            summary: "all shortcuts back on their defaults (2 commands)",
        });
        expect(planAgentReset([], { all: true }, "other")).toEqual({ changed: false, reason: "no shortcut is changed" });
    });

    it("clears entries that did not apply, and says that rather than counting 0 commands", () => {
        const entries = [{ command: "no:such", key: "ctrl+alt+9" }, { command: "tab:new", key: "ctrl+shift+" }];
        setUserKeybindings(entries, () => false);
        const plan = planAgentReset(entries, { all: true }, "other");
        expect(plan).toEqual({ changed: true, count: 2, entries: [], keys: [], summary: "the shortcuts setting cleared (2 entries that did not apply)" });
    });
});
