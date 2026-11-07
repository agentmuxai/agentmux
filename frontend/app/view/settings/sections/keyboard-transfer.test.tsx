// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Export and Import in Settings → Keyboard shortcuts as the person meets them
// (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §9).

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ writes: [] as Array<[string, unknown]>, copied: [] as string[] }));

vi.mock("@/util/platformutil", () => ({ isMacOS: () => false, isLinux: () => false }));
vi.mock("@/app/store/keymodel", () => ({ disableGlobalKeybindings: () => {}, enableGlobalKeybindings: () => {} }));
vi.mock("@/util/clipboard", () => ({ writeText: (t: string) => (h.copied.push(t), Promise.resolve()) }));
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
import { KeyboardSection } from "./keyboard-section";

const setSettings = (global as unknown as { __setSettings: (v: unknown) => void }).__setSettings;

function useEntries(entries: unknown[]): void {
    setSettings({ keybindings: entries });
    setUserKeybindings(entries, () => false);
}

const button = (name: string | RegExp) => screen.getByRole("button", { name });
const paste = (text: string) => fireEvent.input(screen.getByLabelText("Keybindings to import"), { target: { value: text } });
const preview = () => screen.getByRole("group", { name: "Import keybindings" }).querySelector(".kb-import-preview")!.textContent;

const MINE = [
    { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
    { command: "tab:new", key: "ctrl+alt+t", platform: "other" },
];

beforeEach(() => {
    h.writes.length = 0;
    h.copied.length = 0;
    useEntries([]);
});
afterEach(cleanup);

describe("Export", () => {
    it("copies the keybindings array and says how many entries", async () => {
        useEntries(MINE);
        render(() => <KeyboardSection />);
        fireEvent.click(button("Export"));
        await waitFor(() => expect(screen.getByText("Copied 2 entries.")).toBeTruthy());
        expect(JSON.parse(h.copied[0])).toEqual(MINE);
    });

    it("says there is nothing to export when nothing changed", () => {
        render(() => <KeyboardSection />);
        fireEvent.click(button("Export"));
        expect(screen.getByText("Nothing to export: you have no changes.")).toBeTruthy();
        expect(h.copied).toEqual([]);
    });
});

describe("Import", () => {
    it("says why pasted text cannot be read", () => {
        render(() => <KeyboardSection />);
        fireEvent.click(button("Import…"));
        paste("[{");
        expect(preview()).toMatch(/^That is not valid JSON/);
        expect((button("Merge") as HTMLButtonElement).disabled).toBe(true);
    });

    it("says what it would change before anything is written", () => {
        useEntries(MINE);
        render(() => <KeyboardSection />);
        fireEvent.click(button("Import…"));
        paste('[{"command":"-tab:new","platform":"other"},{"command":"tab:new","key":"ctrl+shift+y","platform":"other"},{"command":"tab:ner","key":"x"}]');
        expect(preview()).toBe("2 entries to import; 1 command with keys of your own that merging replaces; 1 not usable, skipped.");
        expect(screen.getByText('Entry 2: unknown command "tab:ner"')).toBeTruthy();
        expect(h.writes).toEqual([]);
    });

    it("merges: the commands it names are taken from the import, the rest stay", () => {
        useEntries([...MINE, { command: "-pane:close", platform: "other" }]);
        render(() => <KeyboardSection />);
        fireEvent.click(button("Import…"));
        paste('[{"command":"-tab:new","platform":"other"},{"command":"tab:new","key":"ctrl+shift+y","platform":"other"}]');
        fireEvent.click(button("Merge"));
        expect(h.writes).toEqual([
            [
                "keybindings",
                [
                    { command: "-pane:close", platform: "other" },
                    { command: "-tab:new", platform: "other" },
                    { command: "tab:new", key: "ctrl+shift+y", platform: "other" },
                ],
            ],
        ]);
        expect(screen.getByText("Imported 2 entries.")).toBeTruthy();
        expect(screen.queryByRole("group", { name: "Import keybindings" })).toBeNull();
    });

    it("replaces everything only on a second click", () => {
        useEntries(MINE);
        render(() => <KeyboardSection />);
        fireEvent.click(button("Import…"));
        paste('[{"command":"-tab:close","platform":"other"}]');
        fireEvent.click(button("Replace all"));
        expect(h.writes).toEqual([]);
        fireEvent.click(button("Click again to replace all"));
        expect(h.writes).toEqual([["keybindings", [{ command: "-tab:close", platform: "other" }]]]);
    });

    it("closes without writing on Cancel", () => {
        render(() => <KeyboardSection />);
        fireEvent.click(button("Import…"));
        fireEvent.click(button("Cancel"));
        expect(screen.queryByRole("group", { name: "Import keybindings" })).toBeNull();
        expect(h.writes).toEqual([]);
    });
});
