// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Search by key in Settings → Keyboard shortcuts: Record keys captures a press and keeps the list to the commands
// that have it (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §4.5).

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ off: vi.fn(), on: vi.fn() }));

vi.mock("@/util/platformutil", () => ({ isMacOS: () => false, isLinux: () => false }));
vi.mock("@/app/store/keymodel", () => ({ disableGlobalKeybindings: h.off, enableGlobalKeybindings: h.on }));
vi.mock("@/util/clipboard", () => ({ writeText: () => Promise.resolve() }));
vi.mock("@/app/store/global", async () => {
    const { createSignal } = await import("solid-js");
    const [settings, setSettings] = createSignal<unknown>({});
    return { settingsAtom: settings, __setSettings: setSettings };
});
vi.mock("../settings-controls", async (orig) => {
    const actual = await orig<typeof import("../settings-controls")>();
    return { ...actual, set: () => {} };
});

import { setUserKeybindings } from "@/app/keybindings/registry";
import { keysStartWith } from "./keyboard-section-state";
import { KeyboardSection } from "./keyboard-section";

const shown = () => [...document.querySelectorAll<HTMLElement>("[data-command]")].map((el) => el.dataset.command);
const button = (name: string | RegExp) => screen.getByRole("button", { name });
const press = (el: Element, key: string, code: string, mods: KeyboardEventInit = {}) => fireEvent.keyDown(el, { key, code, ...mods });

beforeEach(() => {
    h.off.mockClear();
    h.on.mockClear();
    setUserKeybindings([], () => false);
});
afterEach(cleanup);

describe("keysStartWith", () => {
    it("matches a key whatever its spelling", () => {
        expect(keysStartWith(["ctrl+shift+t"], "ctrl+shift+t", "other")).toBe(true);
        expect(keysStartWith(["mod+t"], "ctrl+t", "other")).toBe(true);
        expect(keysStartWith(["mod+t"], "meta+t", "mac")).toBe(true);
        expect(keysStartWith(["ctrl+shift+t"], "ctrl+t", "other")).toBe(false);
    });

    it("finds the chords that start with a key, but not the other way round", () => {
        expect(keysStartWith(["ctrl+k ctrl+s"], "ctrl+k", "other")).toBe(true);
        expect(keysStartWith(["ctrl+k ctrl+s"], "ctrl+k ctrl+s", "other")).toBe(true);
        expect(keysStartWith(["ctrl+k ctrl+s"], "ctrl+s", "other")).toBe(false);
        expect(keysStartWith(["ctrl+k"], "ctrl+k ctrl+s", "other")).toBe(false);
    });
});

describe("Record keys", () => {
    it("keeps the list to the commands that have the pressed key, with the global keys off meanwhile", () => {
        render(() => <KeyboardSection />);
        fireEvent.click(button("Record keys"));
        const field = screen.getByLabelText("Keys to look for") as HTMLInputElement;
        expect(document.activeElement).toBe(field);
        expect(h.off).toHaveBeenCalledTimes(1);
        expect(press(field, "T", "KeyT", { ctrlKey: true, shiftKey: true })).toBe(false);
        expect(field.value).toBe("Ctrl+Shift+T");
        expect(shown()).toContain("tab:new");
        expect(shown()).not.toContain("split:right");
    });

    it("keeps the filter after it stops, shows it, and clears it", () => {
        render(() => <KeyboardSection />);
        fireEvent.click(button("Record keys"));
        press(screen.getByLabelText("Keys to look for"), "T", "KeyT", { ctrlKey: true, shiftKey: true });
        fireEvent.click(button("Stop recording"));
        expect(h.on).toHaveBeenCalledTimes(1);
        expect(screen.getByText("Keys: Ctrl+Shift+T")).toBeTruthy();
        expect(shown()).toContain("tab:new");
        fireEvent.click(button("Clear the key filter"));
        expect(screen.queryByText("Keys: Ctrl+Shift+T")).toBeNull();
        expect(shown()).toContain("split:right");
    });

    it("takes a second press as a chord, a third starts over, and Escape stops", () => {
        render(() => <KeyboardSection />);
        fireEvent.click(button("Record keys"));
        const field = screen.getByLabelText("Keys to look for") as HTMLInputElement;
        press(field, "k", "KeyK", { ctrlKey: true });
        press(field, "n", "KeyN", { ctrlKey: true });
        expect(field.value).toBe("Ctrl+K then Ctrl+N");
        press(field, "m", "KeyM", { ctrlKey: true });
        expect(field.value).toBe("Ctrl+M");
        press(field, "Escape", "Escape");
        expect(screen.queryByLabelText("Keys to look for")).toBeNull();
        expect(h.on).toHaveBeenCalledTimes(1);
    });

    it("lets a bare Tab move on and ignores a modifier on its own", () => {
        render(() => <KeyboardSection />);
        fireEvent.click(button("Record keys"));
        const field = screen.getByLabelText("Keys to look for") as HTMLInputElement;
        expect(press(field, "Tab", "Tab")).toBe(true);
        press(field, "Control", "ControlLeft", { ctrlKey: true });
        expect(field.value).toBe("");
    });
});
