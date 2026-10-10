// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { lastResolvedCommand, listShortcuts, noteResolved, planKeyPress, registerPaneCommandRunner, runCommand, type RunDeps } from "./app-api";
import { DEFAULT_KEYBINDINGS } from "./defaults";

function deps(over: Partial<RunDeps> = {}): RunDeps & { ran: string[] } {
    const ran: string[] = [];
    return {
        ran,
        platform: "other",
        focusedBlockId: () => "focused",
        focusBlock: (id) => id === "focused" || id === "other-pane",
        runGlobal: (command) => {
            ran.push(command);
            return true;
        },
        ...over,
    };
}

describe("listShortcuts", () => {
    it("lists every non-dev command with its keys as the Help pane shows them", () => {
        const list = listShortcuts("other");
        const commands = new Set(list.map((s) => s.command));
        for (const row of DEFAULT_KEYBINDINGS) {
            if (row.devOnly || !(row.other ?? []).length) continue;
            expect(commands.has(row.command), row.command).toBe(true);
        }
        expect(list.some((s) => DEFAULT_KEYBINDINGS.find((r) => r.command === s.command)?.devOnly)).toBe(false);
    });

    it("gives both the shown keys and the table's syntax", () => {
        const save = listShortcuts("mac").find((s) => s.command === "editor:save");
        expect(save).toMatchObject({ keys: ["⌘S"], raw: ["meta+s"], pane: "editor" });
        const clear = listShortcuts("other").find((s) => s.command === "term:clear");
        expect(clear?.when).toContain("terminalFocus");
    });
});

describe("runCommand", () => {
    it("runs a global command through the dispatcher's handler", () => {
        const d = deps();
        expect(runCommand("term:multiInput", undefined, d)).toEqual({ ran: true });
        expect(d.ran).toEqual(["term:multiInput"]);
    });

    it("refuses a permanent delete", () => {
        const r = runCommand("files:deletePermanently", undefined, deps());
        expect(r.ran).toBe(false);
        expect(r.reason).toMatch(/can't be undone/);
    });

    it("names an unknown command", () => {
        expect(runCommand("nope:nothing", undefined, deps()).reason).toMatch(/unknown command/);
    });

    it("refuses a target that isn't in the active tab", () => {
        expect(runCommand("term:multiInput", "elsewhere", deps()).reason).toMatch(/not in the active tab/);
    });

    it("sends a pane command to that pane's runner, not the dispatcher", () => {
        const seen: string[] = [];
        const off = registerPaneCommandRunner("other-pane", (c) => (seen.push(c), true));
        const d = deps();
        expect(runCommand("files:refresh", "other-pane", d)).toEqual({ ran: true });
        expect(seen).toEqual(["files:refresh"]);
        expect(d.ran).toEqual([]);
        off();
        expect(runCommand("files:refresh", "other-pane", d).reason).toMatch(/doesn't handle/);
    });

    it("sends the terminal's own commands to the focused pane's runner", () => {
        const off = registerPaneCommandRunner("focused", (c) => c === "term:clear");
        expect(runCommand("term:clear", undefined, deps())).toEqual({ ran: true });
        expect(runCommand("term:copy", undefined, deps()).reason).toMatch(/didn't apply/);
        off();
    });

    it("needs a focused pane for a pane command without a target", () => {
        expect(runCommand("editor:find", undefined, deps({ focusedBlockId: () => null })).reason).toMatch(/none is focused/);
    });

    it("keeps a newer runner when an older one unregisters", () => {
        const offOld = registerPaneCommandRunner("focused", () => false);
        const offNew = registerPaneCommandRunner("focused", () => true);
        offOld();
        expect(runCommand("editor:find", undefined, deps())).toEqual({ ran: true });
        offNew();
    });
});

describe("runCommand refusals", () => {
    it("sends pane:close to ClosePane", () => {
        const d = deps();
        expect(runCommand("pane:close", undefined, d).reason).toMatch(/ClosePane/);
        expect(d.ran).toEqual([]);
    });

    it("runs files:trash on every platform, since the Trash can be restored from", () => {
        expect(runCommand("files:trash", "focused", deps({ platform: "mac" })).reason).not.toMatch(/not available/);
        expect(planKeyPress("Delete", "mac")).not.toHaveProperty("reason");
        expect(runCommand("files:trash", "focused", deps()).reason).not.toMatch(/not available/);
    });

    it("runs tab:close, which keeps its own confirmation", () => {
        expect(runCommand("tab:close", undefined, deps())).toEqual({ ran: true });
    });
});

describe("planKeyPress", () => {
    it("sends ⌘ as CDP's meta bit on macOS, and Ctrl elsewhere", () => {
        const mac = planKeyPress("meta+d", "mac");
        expect(mac).toMatchObject({ events: [{ key: "d", code: "KeyD", keyCode: 68, modifiers: 4 }], modifiers: [["meta"]] });
        const other = planKeyPress("ctrl+shift+d", "other");
        expect(other).toMatchObject({ events: [{ key: "D", code: "KeyD", modifiers: 2 | 8 }], modifiers: [["ctrl", "shift"]] });
    });

    it("accepts any spelling of a table key, and names its commands", () => {
        const plan = planKeyPress("shift+ctrl+d", "other");
        expect("commands" in plan && plan.commands).toContain(listShortcuts("other").find((s) => s.raw.includes("ctrl+shift+d"))?.command);
    });

    it("sends both keys of a chord", () => {
        const chord = listShortcuts("other").flatMap((s) => s.raw).find((k) => k.includes(" "));
        if (!chord) return;
        const plan = planKeyPress(chord, "other");
        expect("events" in plan && plan.events.length).toBe(2);
    });

    it("refuses a key that isn't in the table", () => {
        expect(planKeyPress("ctrl+alt+shift+meta+q", "other")).toMatchObject({ reason: expect.stringMatching(/isn't a key in the shortcut table/) });
    });

    it("refuses a key bound to a refused command, whatever else it does", () => {
        // ⌘W is pane:close and, in the Files pane, files:closeTab.
        expect(planKeyPress("meta+w", "mac")).toMatchObject({ reason: expect.stringMatching(/pane:close/) });
        expect(planKeyPress("shift+Delete", "other")).toMatchObject({ reason: expect.stringMatching(/deletePermanently/) });
    });

    it("can send every key the table binds, except the refused ones", () => {
        for (const platform of ["mac", "other"] as const) {
            for (const s of listShortcuts(platform)) {
                for (const k of s.raw) {
                    const plan = planKeyPress(k, platform);
                    if ("reason" in plan) expect(plan.reason, `${platform} ${k}`).toMatch(/not available to agents/);
                }
            }
        }
    });
});

describe("lastResolvedCommand", () => {
    it("reports the latest command a key resolved to, and by whom", () => {
        noteResolved("files:refresh", "files");
        expect(lastResolvedCommand()).toMatchObject({ command: "files:refresh", by: "files" });
    });
});
