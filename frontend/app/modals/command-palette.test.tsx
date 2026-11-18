// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The command palette lists what the user typed: the exact command first, not
// the commands that only look like it, and a typo still finds it. The ranking
// itself is covered by command-palette-search.test.ts; this checks the modal
// uses it.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const cmd = (id: string, label: string, category: string, hidden?: boolean, requires?: string) => ({
        id,
        label,
        category,
        hidden,
        requires,
        execute: () => {},
    });
    return {
        ran: [] as string[],
        agents: [] as { id: string; label: string; category: string; detail?: string; keywords?: string; execute: () => void }[],
        caps: { multiWindow: false } as Record<string, boolean>,
        commands: [
            cmd("tab:new", "New Tab", "Tab"),
            cmd("tab:close", "Close Tab", "Tab"),
            cmd("pane:close", "Close Pane", "Pane"),
            cmd("window:close", "Close Window", "Window"),
            cmd("split:right", "Split Right", "Split"),
            cmd("app:identity", "Close Tab Hidden", "App", true),
            cmd("window:new", "New Window", "Window", false, "multiWindow"),
        ],
    };
});

vi.mock("@/app/store/command-registry", () => ({
    commandRegistry: { all: () => h.commands },
    isCommandAvailable: (c: { requires?: string }) => c.requires == null || h.caps[c.requires] === true,
}));
vi.mock("./palette-agents", () => ({ usePaletteAgents: () => () => h.agents }));
vi.mock("@/app/store/keymodel", () => ({ disableGlobalKeybindings: () => {}, enableGlobalKeybindings: () => {} }));
vi.mock("@/app/keybindings", () => ({ shortcutFor: () => "" }));
vi.mock("@/app/platform/pane-overlay", () => ({
    useHiddenTabMarker: () => () => undefined, usePaneOverlay: () => {} }));

import { CommandPaletteModal } from "./command-palette";

afterEach(() => {
    cleanup();
    h.agents = [];
    h.ran = [];
});

function typeQuery(q: string): string[] {
    render(() => <CommandPaletteModal close={() => {}} />);
    fireEvent.input(screen.getByPlaceholderText("Search commands and agents..."), { target: { value: q } });
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

    it("leaves out a command whose host capability is missing, and lists it when present", () => {
        expect(typeQuery("New Window")).not.toContain("New Window");
        cleanup();
        h.caps.multiWindow = true;
        try {
            expect(typeQuery("New Window")).toContain("New Window");
        } finally {
            h.caps.multiWindow = false;
        }
    });
});

describe("CommandPaletteModal agents", () => {
    const agent = (id: string, label: string, detail: string, keywords = "") => ({
        id: `agent:${id}`,
        label,
        category: "Agent",
        detail,
        keywords,
        execute: () => {
            h.ran.push(id);
        },
    });

    it("lists the agents after the commands when nothing is typed, with their detail", () => {
        h.agents = [agent("a1", "Korp", "Specialist · browser engine · idle")];
        expect(typeQuery("")).toEqual(["Split Right", "Close Window", "Close Tab", "New Tab", "Close Pane", "Korp"]);
        expect(document.querySelector(".command-palette-item-detail")?.textContent).toBe("Specialist · browser engine · idle");
    });

    it("finds an agent by its name, and by a word of its seat", () => {
        h.agents = [
            agent("a1", "Korp", "Specialist · browser engine · idle", "Specialist · browser engine claude idle"),
            agent("a2", "Camper", "Specialist · panes and layout · working", "Specialist · panes and layout claude working"),
        ];
        expect(typeQuery("korp")).toEqual(["Korp"]);
        cleanup();
        expect(typeQuery("browser")).toEqual(["Korp"]);
    });

    it("runs the agent's entry on Enter, closing the palette first", async () => {
        vi.useFakeTimers();
        try {
            h.agents = [agent("a1", "Korp", "idle")];
            const close = vi.fn();
            render(() => <CommandPaletteModal close={close} />);
            const input = screen.getByPlaceholderText("Search commands and agents...");
            fireEvent.input(input, { target: { value: "korp" } });
            fireEvent.keyDown(input, { key: "Enter" });
            expect(close).toHaveBeenCalled();
            await vi.runAllTimersAsync();
            expect(h.ran).toEqual(["a1"]);
        } finally {
            vi.useRealTimers();
        }
    });
});

describe("CommandPaletteModal keyboard scroll", () => {
    const scrolled = vi.fn();
    const original = Element.prototype.scrollIntoView;
    beforeEach(() => {
        scrolled.mockReset();
        Element.prototype.scrollIntoView = function (this: Element, arg?: boolean | ScrollIntoViewOptions) {
            scrolled(this.textContent, arg);
        };
    });
    afterEach(() => {
        Element.prototype.scrollIntoView = original;
    });

    function open() {
        render(() => <CommandPaletteModal close={() => {}} />);
        return screen.getByPlaceholderText("Search commands and agents...");
    }

    it("keeps the selected row in view when Down or Up moves it", () => {
        const input = open();
        fireEvent.keyDown(input, { key: "ArrowDown" });
        expect(scrolled).toHaveBeenLastCalledWith(expect.stringContaining("Close Window"), { block: "nearest" });
        fireEvent.keyDown(input, { key: "ArrowDown" });
        fireEvent.keyDown(input, { key: "ArrowUp" });
        expect(scrolled).toHaveBeenLastCalledWith(expect.stringContaining("Close Window"), { block: "nearest" });
    });

    it("does not scroll when the pointer selects a row", () => {
        open();
        const row = document.querySelectorAll(".command-palette-item")[2];
        fireEvent.mouseMove(row, { clientX: 10, clientY: 10 });
        fireEvent.mouseMove(row, { clientX: 12, clientY: 10 });
        expect(row.classList.contains("selected")).toBe(true);
        expect(scrolled).not.toHaveBeenCalled();
    });

    it("keeps the keyboard's row when the list scrolls a different row under a pointer at rest", () => {
        const input = open();
        const rows = () => document.querySelectorAll(".command-palette-item");
        fireEvent.mouseMove(rows()[3], { clientX: 50, clientY: 90 });
        fireEvent.keyDown(input, { key: "ArrowDown" });
        fireEvent.keyDown(input, { key: "ArrowDown" });
        // The list scrolled: the browser reports the same pointer position over another row (no new position).
        fireEvent.mouseMove(rows()[4], { clientX: 50, clientY: 90 });
        fireEvent.mouseEnter(rows()[4]);
        expect(rows()[2].classList.contains("selected")).toBe(true);
        expect(rows()[4].classList.contains("selected")).toBe(false);
    });

    it("shows the very top of the list when Up reaches the first row", () => {
        const input = open();
        const list = document.querySelector(".command-palette-list") as HTMLElement;
        fireEvent.keyDown(input, { key: "ArrowDown" });
        list.scrollTop = 4;
        fireEvent.keyDown(input, { key: "ArrowUp" });
        expect(list.scrollTop).toBe(0);
    });

    it("moves by a page with PageDown and PageUp, and to the ends with End and Home, keeping the row in view", () => {
        const input = open();
        const selected = () => document.querySelector(".command-palette-item.selected .command-palette-item-label")?.textContent;
        const last = document.querySelectorAll(".command-palette-item").length - 1;
        fireEvent.keyDown(input, { key: "End" });
        expect(selected()).toBe("Close Pane");
        expect(scrolled).toHaveBeenLastCalledWith(expect.stringContaining("Close Pane"), { block: "nearest" });
        fireEvent.keyDown(input, { key: "Home" });
        expect(selected()).toBe("Split Right");
        fireEvent.keyDown(input, { key: "PageDown" }); // jsdom has no layout: a page is 8 rows, held to the last
        expect(document.querySelectorAll(".command-palette-item")[last].classList.contains("selected")).toBe(true);
        fireEvent.keyDown(input, { key: "PageUp" });
        expect(selected()).toBe("Split Right");
    });

    it("shows the top of the list again when the search changes", () => {
        const input = open();
        const list = document.querySelector(".command-palette-list") as HTMLElement;
        list.scrollTop = 400;
        fireEvent.input(input, { target: { value: "close" } });
        expect(list.scrollTop).toBe(0);
    });
});
