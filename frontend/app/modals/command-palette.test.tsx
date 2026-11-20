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
        keys: {} as Record<string, string>,
        clipEls: [] as (() => HTMLElement | null | undefined)[],
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
vi.mock("@/app/keybindings", () => ({ shortcutFor: (id: string) => h.keys[id] ?? "" }));
vi.mock("@/app/platform/pane-overlay", () => ({
    useHiddenTabMarker: () => () => undefined,
    usePaneOverlay: (getEl: () => HTMLElement | null | undefined) => {
        h.clipEls.push(getEl);
    },
}));

import { CommandPaletteModal } from "./command-palette";

afterEach(() => {
    cleanup();
    h.agents = [];
    h.ran = [];
    h.keys = {};
    h.clipEls = [];
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

    it("does not scroll, and does not select, when the pointer moves over a row", () => {
        open();
        const row = document.querySelectorAll(".command-palette-item")[2];
        fireEvent.mouseMove(row, { clientX: 10, clientY: 10 });
        fireEvent.mouseMove(row, { clientX: 12, clientY: 10 });
        expect(row.classList.contains("selected")).toBe(false);
        expect(document.querySelectorAll(".command-palette-item")[0].classList.contains("selected")).toBe(true);
        expect(scrolled).not.toHaveBeenCalled();
    });

    it("keeps the keyboard's row when the pointer rests over another", () => {
        const input = open();
        const rows = () => document.querySelectorAll(".command-palette-item");
        fireEvent.mouseMove(rows()[3], { clientX: 50, clientY: 90 });
        fireEvent.mouseEnter(rows()[3]);
        fireEvent.keyDown(input, { key: "ArrowDown" });
        fireEvent.keyDown(input, { key: "ArrowDown" });
        // The list scrolled: the browser reports the same pointer position over another row (no new position).
        fireEvent.mouseMove(rows()[4], { clientX: 50, clientY: 90 });
        fireEvent.mouseEnter(rows()[4]);
        expect(rows()[2].classList.contains("selected")).toBe(true);
        expect(rows()[3].classList.contains("selected")).toBe(false);
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

describe("CommandPaletteModal place and backdrop", () => {
    const open = () => render(() => <CommandPaletteModal close={() => {}} />);

    it("hangs from the top edge, with no backdrop", () => {
        open();
        const root = document.querySelector(".modal-root") as HTMLElement;
        expect(root.getAttribute("data-placement")).toBe("dropdown");
        expect(root.getAttribute("data-backdrop")).toBe("none");
    });

    it("closes on a click outside the panel, and on nothing else", () => {
        const close = vi.fn();
        render(() => <CommandPaletteModal close={close} />);
        fireEvent.pointerDown(document.querySelector(".command-palette-panel")!);
        fireEvent.click(document.querySelector(".command-palette-panel")!);
        expect(close).not.toHaveBeenCalled();
        const backdrop = document.querySelector(".modal-backdrop") as HTMLElement;
        fireEvent.pointerDown(backdrop);
        fireEvent.click(backdrop);
        expect(close).toHaveBeenCalledTimes(1);
    });

    it("cuts only the panel's own rectangle out of a browser pane, not the window's", () => {
        open();
        const cut = h.clipEls.map((get) => get()).filter(Boolean) as HTMLElement[];
        expect(cut).toHaveLength(1);
        expect(cut[0].classList.contains("command-palette-panel")).toBe(true);
    });
});

describe("CommandPaletteModal rows", () => {
    const open = () => render(() => <CommandPaletteModal close={() => {}} />);
    const rowOf = (label: string) =>
        Array.from(document.querySelectorAll<HTMLElement>(".command-palette-item")).find(
            (r) => r.querySelector(".command-palette-item-label")?.textContent === label
        )!;

    it("shows no key cap, and nothing at the right, on a row with no shortcut", () => {
        open();
        const row = rowOf("Close Tab");
        expect(row.querySelector(".command-palette-item-shortcut")).toBeNull();
        expect(row.querySelector("kbd")).toBeNull();
    });

    it("draws a shortcut as one cap per key", () => {
        h.keys["tab:new"] = "Ctrl+Shift+T";
        open();
        const caps = Array.from(rowOf("New Tab").querySelectorAll("kbd")).map((k) => k.textContent);
        expect(caps).toEqual(["Ctrl", "Shift", "T"]);
    });

    it("draws the two steps of a chord as two groups of caps", () => {
        h.keys["split:right"] = "Ctrl+Shift+S then Right";
        open();
        const steps = Array.from(rowOf("Split Right").querySelectorAll(".command-palette-item-step"));
        expect(steps.map((st) => Array.from(st.querySelectorAll("kbd")).map((k) => k.textContent))).toEqual([
            ["Ctrl", "Shift", "S"],
            ["Right"],
        ]);
    });

    it("says no category on a row, only a group's name once, on its first row, in lower case", () => {
        open();
        const groups = Array.from(document.querySelectorAll(".command-palette-item")).map(
            (r) => r.querySelector(".command-palette-item-group")?.textContent ?? ""
        );
        // Split Right | Close Window | Close Tab, New Tab | Close Pane
        expect(groups).toEqual(["split", "window", "tab", "", "pane"]);
        expect(document.querySelector(".command-palette-item-category")).toBeNull();
    });

    it("draws a hairline above each group but the first", () => {
        open();
        const rows = Array.from(document.querySelectorAll(".command-palette-item"));
        expect(rows.map((r) => r.classList.contains("group-start"))).toEqual([false, true, true, false, true]);
    });

    it("names no group while something is typed", () => {
        open();
        fireEvent.input(screen.getByPlaceholderText("Search commands and agents..."), { target: { value: "close" } });
        expect(document.querySelector(".command-palette-item-group")).toBeNull();
        expect(document.querySelector(".command-palette-item.group-start")).toBeNull();
    });

    it("gives a label the row cuts its full text as a tooltip, and a whole one none", () => {
        const scroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollWidth");
        const client = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
        // jsdom has no layout: say the labels of "Close Tab" are cut and the others whole.
        Object.defineProperty(HTMLElement.prototype, "scrollWidth", {
            configurable: true,
            get(this: HTMLElement) {
                return this.textContent === "Close Tab" ? 300 : 10;
            },
        });
        Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 100 });
        try {
            open();
            expect(rowOf("Close Tab").getAttribute("data-hover-tip")).toBe("Close Tab");
            expect(rowOf("New Tab").hasAttribute("data-hover-tip")).toBe(false);
            expect(rowOf("Close Tab").hasAttribute("title")).toBe(false);
        } finally {
            if (scroll) Object.defineProperty(HTMLElement.prototype, "scrollWidth", scroll);
            else delete (HTMLElement.prototype as any).scrollWidth;
            if (client) Object.defineProperty(HTMLElement.prototype, "clientWidth", client);
            else delete (HTMLElement.prototype as any).clientWidth;
        }
    });

    it("runs the row that is clicked, whichever the keyboard selected", async () => {
        vi.useFakeTimers();
        try {
            const close = vi.fn();
            h.commands[3].execute = () => {
                h.ran.push("window:close");
            };
            render(() => <CommandPaletteModal close={close} />);
            fireEvent.click(rowOf("Close Window"));
            expect(close).toHaveBeenCalled();
            await vi.runAllTimersAsync();
            expect(h.ran).toEqual(["window:close"]);
        } finally {
            vi.useRealTimers();
        }
    });

    it("says plainly that nothing matches", () => {
        open();
        fireEvent.input(screen.getByPlaceholderText("Search commands and agents..."), { target: { value: "zzzzqqq" } });
        expect(document.querySelector(".command-palette-empty")?.textContent).toBe('Nothing matches "zzzzqqq"');
    });
});
