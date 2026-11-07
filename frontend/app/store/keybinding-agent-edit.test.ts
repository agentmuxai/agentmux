// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// An agent changing the person's shortcuts: what is written, who is told, and what Undo does
// (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §10).

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    entries: { current: [] as unknown[] },
    writes: [] as unknown[],
    notices: [] as Array<Record<string, any>>,
    actions: new Map<string, () => void>(),
    opened: [] as Array<string | undefined>,
}));

vi.mock("@/app/keybindings", () => ({ keyPlatform: () => "other" }));
vi.mock("@/app/store/global", () => ({
    settingsAtom: () => ({ keybindings: h.entries.current }),
    pushNotification: (n: Record<string, any>) => void h.notices.push(n),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetConfigCommand: (_c: unknown, data: { keybindings: unknown[] }) => {
            h.writes.push(data.keybindings);
            // The setting changes as the write lands.
            h.entries.current = data.keybindings;
            return Promise.resolve();
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/notification/usenotification", () => ({ setNotificationAction: (key: string, fn: () => void) => void h.actions.set(key, fn) }));
vi.mock("@/app/store/keyboard-focus-request", () => ({ openKeyboardSettings: (command?: string) => (h.opened.push(command), Promise.resolve()) }));

import { setUserKeybindings } from "@/app/keybindings/registry";
import { agentResetShortcuts, agentSetShortcut } from "./keybinding-agent-edit";

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
    h.entries.current = [];
    h.writes.length = 0;
    h.notices.length = 0;
    h.actions.clear();
    h.opened.length = 0;
    setUserKeybindings([], () => false);
});

describe("agentSetShortcut", () => {
    it("writes the setting and tells the person, with Undo and Show", async () => {
        const r = await agentSetShortcut({ command: "tab:new", bindings: ["ctrl+shift+y"] }, "AgentX");
        expect(r).toEqual({ changed: true, summary: "New tab: Ctrl+Shift+Y", keys: ["ctrl+shift+y"], note: "The person was told and can undo it from the notice." });
        expect(h.writes).toEqual([
            [
                { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
                { command: "tab:new", key: "ctrl+shift+y", platform: "other" },
            ],
        ]);
        expect(h.notices).toHaveLength(1);
        expect(h.notices[0]).toMatchObject({ title: "AgentX changed a shortcut", message: "New tab: Ctrl+Shift+Y", type: "info" });
        expect(h.notices[0].actions.map((a: { label: string }) => a.label)).toEqual(["Undo", "Show"]);
    });

    it("writes nothing and tells no one when it refuses, and says what is in the way", async () => {
        const r = await agentSetShortcut({ command: "tab:new", bindings: ["ctrl+shift+w"] }, "AgentX");
        expect(r).toMatchObject({ changed: false, taken: [{ command: "pane:close", label: "Close pane", key: "ctrl+shift+w" }] });
        expect(h.writes).toEqual([]);
        expect(h.notices).toEqual([]);
    });

    it("opens Settings at the command from Show", async () => {
        await agentSetShortcut({ command: "tab:new", bindings: ["ctrl+shift+y"] }, "AgentX");
        h.actions.get(h.notices[0].actions[1].actionKey)!();
        expect(h.opened).toEqual(["tab:new"]);
    });

    it("puts the old setting back with Undo, and says so", async () => {
        const before = [{ command: "-pane:close", platform: "other" }];
        h.entries.current = before;
        await agentSetShortcut({ command: "tab:new", bindings: ["ctrl+shift+y"] }, "AgentX");
        h.actions.get(h.notices[0].actions[0].actionKey)!();
        await flush();
        expect(h.writes[1]).toEqual(before);
        expect(h.notices[1]).toMatchObject({ title: "Undone 1 change by AgentX" });
    });

    it("leaves the person's own later changes alone when Undo is pressed", async () => {
        await agentSetShortcut({ command: "tab:new", bindings: ["ctrl+shift+y"] }, "AgentX");
        // The person changed another shortcut afterwards.
        h.entries.current = [...(h.entries.current as unknown[]), { command: "-pane:close", platform: "other" }];
        h.actions.get(h.notices[0].actions[0].actionKey)!();
        await flush();
        expect(h.writes).toHaveLength(1);
        expect(h.notices[1]).toMatchObject({ title: "Not undone" });
    });
});

describe("agentResetShortcuts", () => {
    it("puts a command back and tells the person", async () => {
        h.entries.current = [
            { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
            { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
        ];
        setUserKeybindings(h.entries.current, () => false);
        const r = await agentResetShortcuts({ command: "tab:new" }, "AgentX");
        expect(r).toMatchObject({ changed: true, summary: "New tab back on its default: Ctrl+Shift+T" });
        expect(h.writes).toEqual([[]]);
        expect(h.notices[0]).toMatchObject({ title: "AgentX changed a shortcut" });
    });

    it("counts what a reset of everything changed", async () => {
        h.entries.current = [
            { command: "-tab:new", platform: "other" },
            { command: "-tab:close", platform: "other" },
        ];
        setUserKeybindings(h.entries.current, () => false);
        await agentResetShortcuts({ all: true }, "AgentX");
        expect(h.notices[0]).toMatchObject({ title: "AgentX changed 2 shortcuts" });
    });

    it("writes nothing when there is nothing to reset", async () => {
        expect(await agentResetShortcuts({ command: "tab:new" }, "AgentX")).toMatchObject({ changed: false });
        expect(h.writes).toEqual([]);
    });
});
