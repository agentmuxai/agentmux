// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The command palette lists what the user typed: the exact command first, not
// the commands that only look like it, and a typo still finds it. The ranking
// itself is covered by command-palette-search.test.ts; this checks the modal
// uses it.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const cmd = (id: string, label: string, category: string, hidden?: boolean) => ({
        id,
        label,
        category,
        hidden,
        execute: () => {},
    });
    return {
        commands: [
            cmd("tab:new", "New Tab", "Tab"),
            cmd("tab:close", "Close Tab", "Tab"),
            cmd("pane:close", "Close Pane", "Pane"),
            cmd("window:close", "Close Window", "Window"),
            cmd("split:right", "Split Right", "Split"),
            cmd("app:identity", "Close Tab Hidden", "App", true),
        ],
    };
});

vi.mock("@/app/store/command-registry", () => ({ commandRegistry: { all: () => h.commands } }));
vi.mock("@/app/store/keymodel", () => ({ disableGlobalKeybindings: () => {}, enableGlobalKeybindings: () => {} }));
vi.mock("@/app/keybindings", () => ({ shortcutFor: () => "" }));
vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: () => {} }));

import { CommandPaletteModal } from "./command-palette";

afterEach(() => cleanup());

function typeQuery(q: string): string[] {
    render(() => <CommandPaletteModal close={() => {}} />);
    fireEvent.input(screen.getByPlaceholderText("Search commands..."), { target: { value: q } });
    return Array.from(document.querySelectorAll(".command-palette-item-label")).map((el) => el.textContent ?? "");
}

describe("CommandPaletteModal search", () => {
    it("lists every visible command, in category order, before anything is typed", () => {
        expect(typeQuery("")).toEqual(["Split Right", "Close Window", "Close Tab", "New Tab", "Close Pane"]);
    });

    it("lists an exact command name alone, not the commands that only look like it", () => {
        expect(typeQuery("Close Tab")).toEqual(["Close Tab"]);
    });

    it("still finds a command through a typo", () => {
        expect(typeQuery("Spilt Right")[0]).toBe("Split Right");
    });
});
