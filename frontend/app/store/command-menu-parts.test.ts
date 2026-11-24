// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    commands: new Map<string, { label: string }>(),
    keys: new Map<string, string>(),
    run: vi.fn((_id: string) => true),
}));

vi.mock("@/app/store/command-registry", () => ({
    commandRegistry: { get: (id: string) => h.commands.get(id), run: h.run },
}));
vi.mock("@/app/keybindings", () => ({ shortcutFor: (id: string) => h.keys.get(id) ?? "" }));

import { commandMenuParts } from "./command-menu-parts";

describe("a menu item's parts, read from the command registry", () => {
    beforeEach(() => {
        h.commands.clear();
        h.keys.clear();
        h.run.mockClear();
        h.commands.set("tab:new", { label: "New Tab" });
    });

    it("takes the name from the command, and the key from the shortcut table each time it is read", () => {
        h.keys.set("tab:new", "Ctrl+Shift+T");
        const parts = commandMenuParts("tab:new");
        expect(parts.label).toBe("New Tab");
        expect(parts.shortcut).toBe("Ctrl+Shift+T");
        h.keys.set("tab:new", "Ctrl+Alt+T");
        expect(parts.shortcut).toBe("Ctrl+Alt+T");
        h.keys.delete("tab:new");
        expect(parts.shortcut).toBe("");
    });

    it("runs the command by its id, as the palette does", () => {
        commandMenuParts("tab:new").run();
        expect(h.run).toHaveBeenCalledExactlyOnceWith("tab:new");
    });

    it("refuses an id the registry does not have, rather than build a menu item that does nothing", () => {
        expect(() => commandMenuParts("tab:neew")).toThrow("tab:neew");
    });
});
