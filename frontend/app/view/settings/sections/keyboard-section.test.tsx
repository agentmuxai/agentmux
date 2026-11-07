// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Settings → Keyboard shortcuts as the person meets it: the list, the search and filters, what Reset, Unbind and
// Reset all write to the `keybindings` setting, and the entries that could not be used
// (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §4).

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ writes: [] as Array<[string, unknown]> }));

vi.mock("@/util/platformutil", () => ({ isMacOS: () => false, isLinux: () => false }));
vi.mock("@/app/store/keymodel", () => ({ disableGlobalKeybindings: () => {}, enableGlobalKeybindings: () => {} }));
vi.mock("@/util/clipboard", () => ({ writeText: () => Promise.resolve() }));
vi.mock("@/app/store/global", async () => {
    const { createSignal } = await import("solid-js");
    const [settings, setSettings] = createSignal<unknown>({});
    return { settingsAtom: settings, __setSettings: setSettings };
});
vi.mock("../settings-controls", async (orig) => {
    const actual = await orig<typeof import("../settings-controls")>();
    return { ...actual, set: (key: string, value: unknown) => void h.writes.push([key, value]) };
});

import { setUserKeybindings } from "@/app/keybindings/registry";
import * as global from "@/app/store/global";
import { requestKeyboardRow } from "@/app/store/keyboard-focus-request";
import { KeyboardSection } from "./keyboard-section";

const setSettings = (global as unknown as { __setSettings: (v: unknown) => void }).__setSettings;

/** What the person has in settings.json: the atom the section reads and the registry the keys come from. */
function useEntries(entries: unknown[], known: (id: string) => boolean = () => false): void {
    setSettings({ keybindings: entries });
    setUserKeybindings(entries, known);
}

const line = (command: string) => document.querySelector<HTMLElement>(`[data-command="${command}"]`)!;
const text = (el: Element | null) => el?.textContent ?? "";
const shown = () => [...document.querySelectorAll<HTMLElement>("[data-command]")].map((el) => el.dataset.command);
const button = (name: string | RegExp) => screen.getByRole("button", { name });

beforeEach(() => {
    h.writes.length = 0;
    useEntries([]);
});
afterEach(cleanup);

describe("the list", () => {
    it("shows every command under its category with its key", () => {
        render(() => <KeyboardSection />);
        expect(screen.getByRole("region", { name: "Tabs & windows" })).toBeTruthy();
        expect(text(line("tab:new"))).toContain("New tab");
        expect(text(line("tab:new"))).toContain("Ctrl+Shift+T");
        expect(text(line("tab:new"))).not.toContain("Changed");
        expect(line("dev:perfHud")).toBeNull();
    });

    it("folds the commands of a help line into one line that opens", () => {
        render(() => <KeyboardSection />);
        expect(document.querySelector('[data-group="Go to tab 1–8"]')).toBeTruthy();
        expect(line("tab:goto:3")).toBeNull();
        fireEvent.click(document.querySelector('[data-group="Go to tab 1–8"] button')!);
        expect(text(line("tab:goto:3"))).toContain("Ctrl+3");
    });
});

describe("search and filters", () => {
    it("narrows by action, by command id and by key", () => {
        render(() => <KeyboardSection />);
        const search = screen.getByLabelText("Search shortcuts by action or key");
        fireEvent.input(search, { target: { value: "split" } });
        expect(shown()).toEqual(expect.arrayContaining(["split:right", "split:down"]));
        expect(shown()).not.toContain("tab:new");
        fireEvent.input(search, { target: { value: "ctrl+shift+t" } });
        expect(shown()).toContain("tab:new");
        expect(shown()).not.toContain("split:right");
        fireEvent.input(search, { target: { value: "tab:close" } });
        expect(shown()).toEqual(["tab:close", "doctab:close"]);
    });

    it("says so when nothing matches", () => {
        render(() => <KeyboardSection />);
        fireEvent.input(screen.getByLabelText("Search shortcuts by action or key"), { target: { value: "zzzz" } });
        expect(screen.getByText("No shortcut matches")).toBeTruthy();
    });

    it("keeps one category", () => {
        render(() => <KeyboardSection />);
        fireEvent.change(screen.getByLabelText("Category"), { target: { value: "Files" } });
        expect(shown().every((c) => c!.startsWith("files:"))).toBe(true);
        expect(shown().length).toBeGreaterThan(5);
    });

    it("keeps only the commands the person changed", () => {
        useEntries([
            { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
            { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
        ]);
        render(() => <KeyboardSection />);
        fireEvent.click(screen.getByRole("switch", { name: "Changed only" }));
        expect(shown()).toEqual(["tab:new"]);
    });
});

describe("a changed command", () => {
    beforeEach(() =>
        useEntries([
            { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
            { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
        ])
    );

    it("shows the person's key, a Changed mark and the default", () => {
        render(() => <KeyboardSection />);
        expect(text(line("tab:new"))).toContain("Ctrl+Alt+T");
        expect(text(line("tab:new"))).toContain("Changed");
        expect(text(line("tab:new"))).toContain("Default: Ctrl+Shift+T");
    });

    it("is put back by its Reset, which drops only its own entries", () => {
        useEntries([
            { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
            { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
            { command: "-pane:close", platform: "other" },
        ]);
        render(() => <KeyboardSection />);
        fireEvent.click(button("Reset New tab"));
        expect(h.writes).toEqual([["keybindings", [{ command: "-pane:close", platform: "other" }]]]);
    });
});

describe("Unbind", () => {
    it("writes the one bare unbind and offers Reset after", () => {
        render(() => <KeyboardSection />);
        fireEvent.click(button("Unbind Close tab"));
        expect(h.writes).toEqual([["keybindings", [{ command: "-tab:close", platform: "other" }]]]);
    });

    it("keeps an opened group open, and its line focused, after a member changes", () => {
        render(() => <KeyboardSection />);
        fireEvent.click(document.querySelector('[data-group="Go to tab 1–8"] button')!);
        const member = line("tab:goto:3");
        fireEvent.click(member.querySelector('button[aria-label^="Unbind"]')!);
        // What saving does: the setting changes and every line is built again from the new view.
        useEntries(h.writes[0][1] as unknown[]);
        expect(line("tab:goto:3")).toBe(member);
        expect(text(member)).toContain("Not set");
        expect(document.activeElement).toBe(member);
    });

    it("shows a command with no key as Not set", () => {
        useEntries([{ command: "-tab:close", platform: "other" }]);
        render(() => <KeyboardSection />);
        expect(text(line("tab:close"))).toContain("Not set");
        expect(screen.queryByRole("button", { name: "Unbind Close tab" })).toBeNull();
        expect(screen.getByRole("button", { name: "Reset Close tab" })).toBeTruthy();
    });
});

describe("Reset all", () => {
    it("is off until something changed", () => {
        render(() => <KeyboardSection />);
        expect((button("Reset all") as HTMLButtonElement).disabled).toBe(true);
    });

    it("asks once more, then empties the setting", () => {
        useEntries([{ command: "-tab:close", platform: "other" }]);
        render(() => <KeyboardSection />);
        fireEvent.click(button("Reset all"));
        expect(h.writes).toEqual([]);
        fireEvent.click(button("Click again to reset all"));
        expect(h.writes).toEqual([["keybindings", []]]);
    });
});

describe("entries that could not be used", () => {
    it("show under their command with a way to remove them", () => {
        useEntries([{ command: "tab:new", key: "a b c" }, { command: "-pane:close", platform: "other" }]);
        render(() => <KeyboardSection />);
        expect(text(line("tab:new"))).toContain("a chord is at most two keys");
        fireEvent.click(button("Remove entry 0"));
        expect(h.writes).toEqual([["keybindings", [{ command: "-pane:close", platform: "other" }]]]);
    });

    it("of a closed group show on the group line", () => {
        useEntries([{ command: "tab:goto:3", key: "a b c" }]);
        render(() => <KeyboardSection />);
        expect(line("tab:goto:3")).toBeNull();
        expect(text(document.querySelector('[data-group="Go to tab 1–8"]')!.parentElement)).toContain("a chord is at most two keys");
        fireEvent.click(button("Remove entry 0"));
        expect(h.writes).toEqual([["keybindings", []]]);
    });

    it("of a developer-only command go under Other entries, where they can be removed", () => {
        useEntries([{ command: "dev:perfHud", key: "a b c" }]);
        render(() => <KeyboardSection />);
        expect(text(screen.getByRole("region", { name: "Other entries" }))).toContain("a chord is at most two keys");
        fireEvent.click(button("Remove entry 0"));
        expect(h.writes).toEqual([["keybindings", []]]);
    });

    it("go under Other entries when no command in the list is named", () => {
        useEntries([{ command: "tab:ner", key: "ctrl+alt+n" }]);
        render(() => <KeyboardSection />);
        const other = screen.getByRole("region", { name: "Other entries" });
        expect(text(other)).toContain('unknown command "tab:ner"');
        fireEvent.click(button("Remove entry 0"));
        expect(h.writes).toEqual([["keybindings", []]]);
    });
});

describe("the keyboard", () => {
    it("walks the lines with the arrow keys and jumps to the search with /", () => {
        render(() => <KeyboardSection />);
        const first = document.querySelector<HTMLElement>("[data-kb-row]")!;
        first.focus();
        fireEvent.keyDown(first, { key: "ArrowDown" });
        const second = document.querySelectorAll<HTMLElement>("[data-kb-row]")[1];
        expect(document.activeElement).toBe(second);
        fireEvent.keyDown(second, { key: "ArrowUp" });
        expect(document.activeElement).toBe(first);
        fireEvent.keyDown(first, { key: "/" });
        expect(document.activeElement).toBe(screen.getByLabelText("Search shortcuts by action or key"));
    });

    it("leaves Ctrl+/ and Cmd+/ to Help's shortcut", () => {
        render(() => <KeyboardSection />);
        const first = document.querySelector<HTMLElement>("[data-kb-row]")!;
        first.focus();
        expect(fireEvent.keyDown(first, { key: "/", ctrlKey: true })).toBe(true);
        expect(fireEvent.keyDown(first, { key: "/", metaKey: true })).toBe(true);
        expect(document.activeElement).toBe(first);
    });
});

describe("Help's links", () => {
    it("scroll to the command that was asked for and show everything", async () => {
        const scroll = vi.fn();
        Element.prototype.scrollIntoView = scroll;
        render(() => <KeyboardSection />);
        fireEvent.input(screen.getByLabelText("Search shortcuts by action or key"), { target: { value: "split" } });
        requestKeyboardRow("tab:new");
        await Promise.resolve();
        expect(scroll).toHaveBeenCalled();
        expect(document.activeElement).toBe(line("tab:new"));
        expect((screen.getByLabelText("Search shortcuts by action or key") as HTMLInputElement).value).toBe("");
    });

    it("open the group of a command that has no line of its own until then", async () => {
        Element.prototype.scrollIntoView = vi.fn();
        render(() => <KeyboardSection />);
        expect(line("tab:goto:3")).toBeNull();
        requestKeyboardRow("tab:goto:3");
        await Promise.resolve();
        expect(document.activeElement).toBe(line("tab:goto:3"));
    });
});
