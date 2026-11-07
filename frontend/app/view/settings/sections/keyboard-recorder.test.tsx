// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Recording a shortcut in Settings → Keyboard shortcuts as the person meets it: press the keys, see what they
// collide with, save (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §4.5, §5).

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ writes: [] as Array<[string, unknown]>, off: vi.fn(), on: vi.fn() }));

vi.mock("@/util/platformutil", () => ({ isMacOS: () => false, isLinux: () => false }));
vi.mock("@/app/store/keymodel", () => ({ disableGlobalKeybindings: h.off, enableGlobalKeybindings: h.on }));
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

const line = (command: string) => document.querySelector<HTMLElement>(`[data-command="${command}"]`)!;
const button = (name: string | RegExp) => screen.getByRole("button", { name });
const field = (label: string) => screen.getByLabelText(`New shortcut for ${label}`) as HTMLInputElement;
const recorder = (label: string) => screen.queryByRole("group", { name: `Set the shortcut for ${label}` });

/** A key press as the browser reports it. */
const press = (el: Element, key: string, code: string, mods: KeyboardEventInit = {}) => fireEvent.keyDown(el, { key, code, ...mods });

/** Opens the recorder of New tab and returns its field. */
function open(label = "New tab"): HTMLInputElement {
    fireEvent.click(button(`Edit keys of ${label}`));
    return field(label);
}

beforeEach(() => {
    h.writes.length = 0;
    h.off.mockClear();
    h.on.mockClear();
    useEntries([]);
});
afterEach(cleanup);

describe("opening", () => {
    it("opens under the line, takes the focus and turns the global keys off", () => {
        render(() => <KeyboardSection />);
        const input = open();
        expect(recorder("New tab")).toBeTruthy();
        expect(document.activeElement).toBe(input);
        expect(h.off).toHaveBeenCalledTimes(1);
        expect(document.body.textContent).toContain("Press the new shortcut for New tab. Escape cancels.");
    });

    it("opens with Enter on the line itself", () => {
        render(() => <KeyboardSection />);
        const row = line("tab:new");
        row.focus();
        fireEvent.keyDown(row, { key: "Enter" });
        expect(recorder("New tab")).toBeTruthy();
    });

    it("turns the global keys back on and gives the line its focus when it closes", () => {
        render(() => <KeyboardSection />);
        const input = open();
        press(input, "Escape", "Escape");
        expect(recorder("New tab")).toBeNull();
        expect(h.on).toHaveBeenCalledTimes(1);
        expect(document.activeElement).toBe(line("tab:new"));
    });

    it("keeps the global keys off until the last recorder is closed", () => {
        render(() => <KeyboardSection />);
        const first = open("New tab");
        open("Close tab");
        expect(h.off).toHaveBeenCalledTimes(1);
        press(first, "Escape", "Escape");
        expect(recorder("New tab")).toBeNull();
        expect(recorder("Close tab")).toBeTruthy();
        expect(h.on).not.toHaveBeenCalled();
        fireEvent.click(button("Cancel"));
        expect(h.on).toHaveBeenCalledTimes(1);
    });

    it("labels the button Set key for a command with no key", () => {
        useEntries([{ command: "-tab:close", platform: "other" }]);
        render(() => <KeyboardSection />);
        expect(button("Set key for Close tab").textContent).toBe("Set key");
    });
});

describe("pressing keys", () => {
    it("shows the press, and keeps the press from reaching anything else", () => {
        render(() => <KeyboardSection />);
        const input = open();
        expect(press(input, "y", "KeyY", { ctrlKey: true, shiftKey: true })).toBe(false);
        expect(input.value).toBe("Ctrl+Shift+Y");
    });

    it("ignores a modifier on its own and lets a bare Tab move on", () => {
        render(() => <KeyboardSection />);
        const input = open();
        press(input, "Control", "ControlLeft", { ctrlKey: true });
        expect(input.value).toBe("");
        expect(press(input, "Tab", "Tab")).toBe(true);
        expect(press(input, "Tab", "Tab", { shiftKey: true })).toBe(true);
        expect(input.value).toBe("");
    });

    it("makes a second press a two-step shortcut, and a third starts over", () => {
        render(() => <KeyboardSection />);
        const input = open();
        press(input, "k", "KeyK", { ctrlKey: true });
        press(input, "n", "KeyN", { ctrlKey: true });
        expect(input.value).toBe("Ctrl+K then Ctrl+N");
        press(input, "m", "KeyM", { ctrlKey: true });
        expect(input.value).toBe("Ctrl+M");
    });

    it("records Enter only through its button, and Clear starts again", () => {
        render(() => <KeyboardSection />);
        const input = open();
        press(input, "Enter", "Enter");
        expect(input.value).toBe("");
        fireEvent.click(button("Enter"));
        expect(input.value).toBe("Enter");
        fireEvent.click(button("Clear"));
        expect(input.value).toBe("");
    });
});

describe("saving", () => {
    it("writes the diff from the defaults with Save", () => {
        render(() => <KeyboardSection />);
        press(open(), "y", "KeyY", { ctrlKey: true, shiftKey: true });
        fireEvent.click(button("Save"));
        expect(h.writes).toEqual([
            [
                "keybindings",
                [
                    { command: "-tab:new", key: "ctrl+shift+t", platform: "other" },
                    { command: "tab:new", key: "ctrl+shift+y", platform: "other" },
                ],
            ],
        ]);
        expect(recorder("New tab")).toBeNull();
    });

    it("saves with Enter when nothing stands in the way", () => {
        render(() => <KeyboardSection />);
        const input = open();
        press(input, "Enter", "Enter");
        expect(h.writes).toEqual([]);
        press(input, "y", "KeyY", { ctrlKey: true, shiftKey: true });
        press(input, "Enter", "Enter");
        expect(h.writes).toHaveLength(1);
    });

    it("keeps the current keys too when asked", () => {
        render(() => <KeyboardSection />);
        const input = open();
        fireEvent.click(screen.getByRole("switch", { name: "Keep the current keys too" }));
        press(input, "y", "KeyY", { ctrlKey: true, shiftKey: true });
        fireEvent.click(button("Save"));
        expect(h.writes).toEqual([["keybindings", [{ command: "tab:new", key: "ctrl+shift+y", platform: "other" }]]]);
    });

    it("cancels without writing", () => {
        render(() => <KeyboardSection />);
        press(open(), "y", "KeyY", { ctrlKey: true, shiftKey: true });
        fireEvent.click(button("Cancel"));
        expect(h.writes).toEqual([]);
        expect(recorder("New tab")).toBeNull();
    });
});

describe("the recorder's own controls", () => {
    it("keep their arrow keys: the picker does not send the focus to another line", () => {
        render(() => <KeyboardSection />);
        open();
        const picker = screen.getByLabelText("Where this shortcut works");
        picker.focus();
        expect(fireEvent.keyDown(picker, { key: "ArrowDown" })).toBe(true);
        expect(document.activeElement).toBe(picker);
    });

    it("say why an AltGr character is not recorded", () => {
        render(() => <KeyboardSection />);
        const input = open();
        press(input, "@", "KeyQ", { ctrlKey: true, altKey: true, modifierAltGraph: true });
        expect(input.value).toBe("");
        expect(document.body.textContent).toContain("AltGr types a character on this keyboard");
        press(input, "y", "KeyY", { ctrlKey: true, shiftKey: true });
        expect(document.body.textContent).not.toContain("AltGr types a character on this keyboard");
    });
});

describe("collisions", () => {
    it("names the command that has the key, and takes it only on Replace", () => {
        render(() => <KeyboardSection />);
        const input = open();
        press(input, "W", "KeyW", { ctrlKey: true, shiftKey: true });
        expect(document.body.textContent).toContain("Ctrl+Shift+W is already Close pane, which works in the same places as New tab.");
        expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
        press(input, "Enter", "Enter");
        expect(h.writes).toEqual([]);
        fireEvent.click(button("Replace"));
        const [[, written]] = h.writes as [[string, Array<{ command: string; key?: string }>]];
        expect(written).toContainEqual({ command: "-pane:close", platform: "other" });
        expect(written).toContainEqual({ command: "tab:new", key: "ctrl+shift+w", platform: "other" });
    });

    it("lets the key sit beside it once it works somewhere else", () => {
        render(() => <KeyboardSection />);
        const input = open();
        press(input, "C", "KeyC", { ctrlKey: true, shiftKey: true });
        expect(screen.queryByRole("button", { name: "Replace" })).toBeTruthy();
        fireEvent.change(screen.getByLabelText("Where this shortcut works"), { target: { value: "!terminalFocus" } });
        expect(screen.queryByRole("button", { name: "Replace" })).toBeNull();
        fireEvent.click(button("Save"));
        expect(h.writes[0][1]).toContainEqual({ command: "tab:new", key: "ctrl+shift+c", when: "!terminalFocus", platform: "other" });
    });
});

describe("what the recorder says follows what is recorded", () => {
    it("drops the plain-key refusal as soon as plain keys are allowed", () => {
        render(() => <KeyboardSection />);
        press(open(), "t", "KeyT");
        expect(document.body.textContent).toContain("A plain key would fire while you type");
        fireEvent.click(screen.getByRole("switch", { name: "Allow plain keys" }));
        expect(document.body.textContent).not.toContain("A plain key would fire while you type");
    });

    it("drops a collision when a second key makes the shortcut a different one", () => {
        render(() => <KeyboardSection />);
        const input = open();
        press(input, "W", "KeyW", { ctrlKey: true, shiftKey: true });
        expect(document.body.textContent).toContain("is already Close pane");
        press(input, "k", "KeyK", { ctrlKey: true });
        expect(input.value).toBe("Ctrl+Shift+W then Ctrl+K");
        expect(document.body.textContent).not.toContain("is already Close pane");
        // Now it is a chord whose first step Close pane uses on its own.
        expect(document.body.textContent).toContain("Close pane uses Ctrl+Shift+W on its own, so this two-step shortcut could never start.");
    });
});

describe("keys the system takes", () => {
    it("cannot be saved when the system takes them first", () => {
        render(() => <KeyboardSection />);
        press(open(), "F4", "F4", { altKey: true });
        expect(document.body.textContent).toContain("Windows takes Alt+F4 to close the window");
        expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
    });

    it("waits for Use it anyway when the desktop often takes them", () => {
        render(() => <KeyboardSection />);
        press(open(), "t", "KeyT", { ctrlKey: true, altKey: true });
        expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
        fireEvent.click(screen.getByRole("switch", { name: "Use it anyway" }));
        expect((button("Save") as HTMLButtonElement).disabled).toBe(false);
    });
});

describe("plain keys", () => {
    it("are refused until allowed, then saved outside text fields and terminals only", () => {
        render(() => <KeyboardSection />);
        press(open(), "t", "KeyT");
        expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
        expect(document.body.textContent).toContain("A plain key would fire while you type");
        fireEvent.click(screen.getByRole("switch", { name: "Allow plain keys" }));
        fireEvent.click(button("Save"));
        expect(h.writes[0][1]).toContainEqual({ command: "tab:new", key: "t", when: "!textInputFocus && !terminalFocus", platform: "other" });
    });
});
