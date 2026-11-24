// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The command palette's keystroke path. A power user wants the rows at once, so a typed character must cost no more than
// the rows that changed: the list is searched, not sorted again, and a row that stays is kept, not built again.
// Budget: a typed character to its rows in at most 16 ms of script time, on a list of several thousand entries.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CommandEntry } from "@/app/store/command-registry";
import { searchCommands } from "./command-palette-search";
import { arrangeRecent, paletteRows, paletteSegments, sortCommands } from "./palette-groups";

const h = vi.hoisted(() => ({
    commands: [] as unknown[],
    setAgents: (_: unknown[]) => {},
    sorts: 0,
}));

vi.mock("@/app/store/command-registry", () => ({
    commandRegistry: { all: () => h.commands },
    isCommandAvailable: () => true,
}));
vi.mock("./palette-agents", async () => {
    const { createSignal } = await import("solid-js");
    const [agents, setAgents] = createSignal<unknown[]>([]);
    h.setAgents = setAgents;
    return { usePaletteAgents: () => agents };
});
vi.mock("./palette-groups", async (importOriginal) => {
    const real = await importOriginal<typeof import("./palette-groups")>();
    return {
        ...real,
        sortCommands: (cmds: CommandEntry[]) => {
            h.sorts++;
            return real.sortCommands(cmds);
        },
    };
});
vi.mock("@/app/store/keymodel", () => ({ disableGlobalKeybindings: () => {}, enableGlobalKeybindings: () => {} }));
vi.mock("@/app/keybindings", () => ({ shortcutFor: () => "", isKeyFor: () => false }));
vi.mock("@/app/platform/pane-overlay", () => ({
    useHiddenTabMarker: () => () => undefined,
    usePaneOverlay: () => {},
}));

import { CommandPaletteModal, FIRST_ROWS, ROW_CHUNK } from "./command-palette";

afterEach(() => {
    cleanup();
    h.commands = [];
    h.setAgents([]);
    h.sorts = 0;
});

const CATEGORIES = ["Open", "Split", "Window", "Tab", "Pane", "Agent", "Dev", "View", "Settings"];

function entry(i: number, category = CATEGORIES[i % CATEGORIES.length], prefix = "Command"): CommandEntry {
    return {
        id: `${prefix.toLowerCase()}:${i}`,
        label: `${prefix} ${i} ${["alpha", "bravo", "charlie", "delta", "echo"][i % 5]}`,
        category,
        keywords: `word${i % 17} thing${i % 29}`,
        execute: () => {},
    };
}

const entries = (n: number, prefix?: string, category?: string): CommandEntry[] => Array.from({ length: n }, (_, i) => entry(i, category, prefix));

const rowLabels = () => Array.from(document.querySelectorAll(".command-palette-item-label")).map((el) => el.textContent ?? "");
const rowEl = (label: string): HTMLElement | undefined =>
    Array.from(document.querySelectorAll<HTMLElement>(".command-palette-item")).find((r) => r.querySelector(".command-palette-item-label")?.textContent === label);

function open(): HTMLInputElement {
    render(() => <CommandPaletteModal close={() => {}} />);
    return screen.getByPlaceholderText("Search commands and agents") as HTMLInputElement;
}

const type = (input: HTMLInputElement, value: string) => fireEvent.input(input, { target: { value } });

describe("the palette keeps the rows that stay", () => {
    // "Alpha 1 bravo", "Alpha 2 charlie" and "Beta 3 delta" all contain an a; only the first two contain "alp".
    const three = () => [entry(1, "Tab", "Alpha"), entry(2, "Tab", "Alpha"), entry(3, "Pane", "Beta")];

    it("keeps a row's element when the next character leaves its command in the list", () => {
        h.commands = three();
        const input = open();
        type(input, "a");
        expect(rowLabels()).toEqual(["Alpha 1 bravo", "Alpha 2 charlie", "Beta 3 delta"]);
        const kept = rowEl("Alpha 1 bravo")!;
        const gone = rowEl("Beta 3 delta")!;
        type(input, "alp");
        expect(rowLabels()).toEqual(["Alpha 1 bravo", "Alpha 2 charlie"]);
        expect(rowEl("Alpha 1 bravo")).toBe(kept);
        expect(gone.isConnected).toBe(false);
    });

    it("keeps the element of the group the rows are in, while typing", () => {
        h.commands = three();
        const input = open();
        type(input, "a");
        const group = rowEl("Alpha 1 bravo")!.parentElement;
        type(input, "alp");
        expect(rowEl("Alpha 1 bravo")!.parentElement).toBe(group);
    });

    it("builds a row for a command that comes into the list, and keeps the others", () => {
        h.commands = three();
        const input = open();
        type(input, "alp");
        const kept = rowEl("Alpha 1 bravo")!;
        type(input, "a");
        expect(rowLabels()).toHaveLength(3);
        expect(rowEl("Alpha 1 bravo")).toBe(kept);
    });

    it("updates a kept row in place when its command changes under the same id (an agent's state)", () => {
        const agent = (detail: string): CommandEntry => ({ ...entry(1, "Agent", "Korp"), id: "agent:korp", detail });
        h.setAgents([agent("idle")]);
        open();
        const row = rowEl("Korp 1 bravo")!;
        expect(row.querySelector(".command-palette-item-detail")?.textContent).toBe("idle");
        h.setAgents([agent("working")]);
        expect(rowEl("Korp 1 bravo")).toBe(row);
        expect(row.querySelector(".command-palette-item-detail")?.textContent).toBe("working");
    });

    it("marks the selected row without building the others again", () => {
        h.commands = entries(6, "Cmd", "Tab");
        const input = open();
        const before = Array.from(document.querySelectorAll(".command-palette-item"));
        fireEvent.keyDown(input, { key: "ArrowDown" });
        fireEvent.keyDown(input, { key: "ArrowDown" });
        const after = Array.from(document.querySelectorAll(".command-palette-item"));
        expect(after).toEqual(before);
        expect(after.filter((r) => r.classList.contains("selected"))).toHaveLength(1);
        expect(after[2].classList.contains("selected")).toBe(true);
        expect(after[2].getAttribute("aria-selected")).toBe("true");
    });
});

describe("the palette's keystroke budget", () => {
    // The commands and the agents the heaviest workspace lists: several thousand entries, more than a hundred agents.
    const all = [...entries(5000, "Command"), ...entries(150, "Agent", "Agent")];
    const BUDGET_MS = 16;
    const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];

    it("searches, arranges and groups a character in under 16 ms of script time", () => {
        const sorted = sortCommands(all);
        const queries = ["c", "co", "com", "comm", "command 4", "command 49", "bravo", "word3", "agent 1"];
        const times = queries.map((q) => {
            const t0 = performance.now();
            const found = searchCommands(sorted, q);
            const { cmds, recent } = arrangeRecent(found, ["command:7", "agent:3", "command:42"], true);
            paletteSegments(paletteRows(cmds, true, recent));
            return performance.now() - t0;
        });
        expect(median(times)).toBeLessThan(BUDGET_MS);
    });

    const builtRows = () => document.querySelectorAll(".command-palette-item").length;
    const frame = () => new Promise((r) => requestAnimationFrame(r));

    it("builds only the first rows when the palette opens and when a character matches thousands, not all of them", () => {
        h.commands = all;
        const input = open();
        expect(builtRows()).toBe(FIRST_ROWS);
        type(input, "c");
        expect(builtRows()).toBe(FIRST_ROWS);
    });

    it("builds the rows after the first ones once the first frame is drawn, not in the rendering step that draws it", async () => {
        h.commands = all;
        open();
        // The rendering step that draws the palette: its callbacks run before the draw, so a chunk built there is in the first frame.
        await frame();
        expect(builtRows()).toBe(FIRST_ROWS);
        await vi.waitFor(() => expect(builtRows()).toBeGreaterThan(FIRST_ROWS)); // a chunk a frame follows, as many as there are
    });

    it("builds the rest a chunk after each frame until every row is there, and starts over with the next character", async () => {
        h.commands = all.slice(0, ROW_CHUNK * 3 + 7);
        const input = open();
        expect(builtRows()).toBe(FIRST_ROWS);
        await vi.waitFor(() => expect(builtRows()).toBe(ROW_CHUNK * 3 + 7));
        type(input, "command");
        expect(builtRows()).toBe(FIRST_ROWS);
    });

    it("builds rows that join the list while the palette is open, without a character typed", async () => {
        h.commands = all.slice(0, ROW_CHUNK * 3 + 20);
        open();
        await vi.waitFor(() => expect(builtRows()).toBe(ROW_CHUNK * 3 + 20)); // everything there is has been built, and the building has stopped
        // An agent starts: more rows than were built come into the list.
        h.setAgents(entries(ROW_CHUNK * 2, "Agent", "Agent"));
        await vi.waitFor(() => expect(builtRows()).toBe(ROW_CHUNK * 3 + 20 + ROW_CHUNK * 2));
    });

    it("searches the whole list when a character is typed, before the later chunks are built", () => {
        h.commands = all.slice(0, 500);
        const input = open();
        expect(builtRows()).toBe(FIRST_ROWS);
        // A row far past the first ones (Command 400): still found by the first character typed after the open.
        type(input, "Command 400 alpha");
        expect(rowLabels()).toContain("Command 400 alpha");
    });

    it("moves the selection past the built rows while the later chunks are still to come", () => {
        h.commands = all.slice(0, 500);
        const input = open();
        expect(builtRows()).toBe(FIRST_ROWS);
        for (let i = 0; i < 30; i++) fireEvent.keyDown(input, { key: "ArrowDown" });
        const items = Array.from(document.querySelectorAll(".command-palette-item"));
        expect(items.length).toBeGreaterThan(30);
        expect(items[30].classList.contains("selected")).toBe(true);
    });

    it("builds through the row a key moves the selection to, so End shows the last row", () => {
        h.commands = all.slice(0, 500);
        const input = open();
        fireEvent.keyDown(input, { key: "End" });
        expect(builtRows()).toBeGreaterThanOrEqual(500);
        const last = Array.from(document.querySelectorAll(".command-palette-item")).pop()!;
        expect(last.classList.contains("selected")).toBe(true);
    });

    it("takes a typed character to its rows within the budget, with thousands of entries (jsdom: three times the 16 ms)", () => {
        h.commands = all;
        const input = open();
        const times = ["c", "co", "com", "comm", "comma", "comman", "command"].map((q) => {
            const t0 = performance.now();
            type(input, q);
            return performance.now() - t0;
        });
        expect(median(times)).toBeLessThan(BUDGET_MS * 3);
    });

    it("sorts the list when the palette opens and not again for any character typed", () => {
        h.commands = all;
        const input = open();
        expect(h.sorts).toBe(1);
        for (const q of ["c", "co", "com", "comm", "comma"]) type(input, q);
        expect(h.sorts).toBe(1);
    });

    it("sorts in category order, then by name, as the localeCompare version did", () => {
        const sorted = sortCommands([entry(2, "Tab", "B"), entry(1, "Pane", "A"), entry(3, "Open", "Z"), entry(4, "Tab", "A")]);
        expect(sorted.map((c) => c.label.split(" ")[0] + c.category)).toEqual(["ZOpen", "ATab", "BTab", "APane"]);
    });
});
