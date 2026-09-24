// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * formatKeyDescription renders a keymodel key description ("Cmd:t") as a
 * shortcut label. The label must name exactly the keys checkKeyPressed matches
 * on that platform. The hamburger menu once showed "Ctrl+T" for "Cmd:t" on
 * Windows/Linux, where "Cmd" means Alt, so the label named a key that did nothing.
 */

import { afterEach, describe, expect, it } from "vitest";
import { adaptFromReactOrNativeKeyEvent, checkKeyPressed, formatKeyDescription, setKeyUtilPlatform } from "./keyutil";

type Platform = "darwin" | "win32" | "linux";
const PLATFORMS: Platform[] = ["darwin", "win32", "linux"];

// [description, macOS label, Windows/Linux label]. Descriptions are the forms
// keymodel.ts registers: Cmd vs Ctrl, both modifier orders, brackets, arrows,
// c{...} codes, zoom punctuation, digits, and a bare named key.
const CASES: [string, string, string][] = [
    ["Cmd:t", "⌘T", "Alt+T"],
    ["Ctrl:Shift:n", "⌃⇧N", "Ctrl+Shift+N"],
    ["Ctrl:p", "⌃P", "Ctrl+P"],
    ["Cmd:n", "⌘N", "Alt+N"],
    ["Cmd:]", "⌘]", "Alt+]"],
    ["Cmd:[", "⌘[", "Alt+["],
    ["Shift:Cmd:]", "⇧⌘]", "Alt+Shift+]"],
    ["Cmd:Shift:w", "⇧⌘W", "Alt+Shift+W"],
    ["Shift:Cmd:d", "⇧⌘D", "Alt+Shift+D"],
    ["Ctrl:]", "⌃]", "Ctrl+]"],
    ["Ctrl:Shift:k", "⌃⇧K", "Ctrl+Shift+K"],
    ["Ctrl:Shift:ArrowUp", "⌃⇧↑", "Ctrl+Shift+↑"],
    ["Ctrl:Shift:ArrowDown", "⌃⇧↓", "Ctrl+Shift+↓"],
    ["Ctrl:Shift:ArrowLeft", "⌃⇧←", "Ctrl+Shift+←"],
    ["Ctrl:Shift:ArrowRight", "⌃⇧→", "Ctrl+Shift+→"],
    ["Ctrl:Shift:c{Digit3}", "⌃⇧3", "Ctrl+Shift+3"],
    ["Ctrl:Shift:c{Numpad3}", "⌃⇧Num 3", "Ctrl+Shift+Num 3"],
    ["Cmd:1", "⌘1", "Alt+1"],
    ["Cmd:=", "⌘=", "Alt+="],
    ["Cmd:+", "⌘+", "Alt++"],
    ["Cmd:-", "⌘-", "Alt+-"],
    ["Cmd:0", "⌘0", "Alt+0"],
    ["Ctrl:=", "⌃=", "Ctrl+="],
    ["Ctrl:-", "⌃-", "Ctrl+-"],
    ["Ctrl:0", "⌃0", "Ctrl+0"],
    ["Escape", "⎋", "Esc"],
    ["ArrowDown", "↓", "↓"],
];

// Reads a label the way a user would, and produces the keydown they would make.
// Written independently of the formatter, so a formatter bug can't hide here.
function keydownFromLabel(label: string, platform: Platform): KeyboardEvent {
    const mods = { ctrlKey: false, altKey: false, shiftKey: false, metaKey: false };
    let keyName: string;
    if (platform === "darwin") {
        const glyphs: Record<string, keyof typeof mods> = {
            "⌃": "ctrlKey",
            "⌥": "altKey",
            "⇧": "shiftKey",
            "⌘": "metaKey",
        };
        let idx = 0;
        while (idx < label.length && glyphs[label[idx]] != null) {
            mods[glyphs[label[idx]]] = true;
            idx++;
        }
        keyName = label.slice(idx);
    } else {
        // "Alt++" is Alt plus the "+" key.
        const parts = label.endsWith("++") ? [...label.slice(0, -2).split("+"), "+"] : label.split("+");
        keyName = parts.pop();
        const names: Record<string, keyof typeof mods> = {
            Ctrl: "ctrlKey",
            Alt: "altKey",
            Shift: "shiftKey",
            Win: "metaKey",
            Super: "metaKey",
        };
        for (const part of parts) {
            expect(names[part], `unknown modifier "${part}" in label "${label}"`).toBeDefined();
            mods[names[part]] = true;
        }
    }
    const named: Record<string, string> = {
        "↑": "ArrowUp",
        "↓": "ArrowDown",
        "←": "ArrowLeft",
        "→": "ArrowRight",
        "⎋": "Escape",
        Esc: "Escape",
    };
    let key = named[keyName] ?? keyName;
    let code = "";
    const numpad = keyName.match(/^Num (\d)$/);
    if (numpad != null) {
        key = numpad[1];
        code = `Numpad${numpad[1]}`;
    } else if (/^\d$/.test(keyName)) {
        code = `Digit${keyName}`;
    } else if (/^[A-Z]$/.test(keyName)) {
        code = `Key${keyName}`;
        // A browser reports the shifted character: "N" with Shift, "n" without.
        key = mods.shiftKey ? keyName : keyName.toLowerCase();
    }
    return new KeyboardEvent("keydown", { key, code, ...mods });
}

function labelMatches(label: string, keyDescription: string, platform: Platform): boolean {
    setKeyUtilPlatform(platform);
    return checkKeyPressed(adaptFromReactOrNativeKeyEvent(keydownFromLabel(label, platform)), keyDescription);
}

afterEach(() => {
    // keyutil's module default.
    setKeyUtilPlatform("darwin");
});

describe("formatKeyDescription", () => {
    describe.each(CASES)("%s", (keyDescription, macLabel, otherLabel) => {
        it(`renders "${macLabel}" on macOS`, () => {
            expect(formatKeyDescription(keyDescription, "darwin")).toBe(macLabel);
        });

        it(`renders "${otherLabel}" on Windows and Linux`, () => {
            expect(formatKeyDescription(keyDescription, "win32")).toBe(otherLabel);
            expect(formatKeyDescription(keyDescription, "linux")).toBe(otherLabel);
        });

        it.each(PLATFORMS)("pressing the keys its label names fires the binding (%s)", (platform) => {
            const label = formatKeyDescription(keyDescription, platform);
            expect(labelMatches(label, keyDescription, platform)).toBe(true);
        });
    });

    it("renders Cmd as ⌘ on macOS and Alt on Windows/Linux; Ctrl is Control everywhere", () => {
        expect(formatKeyDescription("Cmd:x", "darwin")).toBe("⌘X");
        expect(formatKeyDescription("Cmd:x", "win32")).toBe("Alt+X");
        expect(formatKeyDescription("Ctrl:x", "darwin")).toBe("⌃X");
        expect(formatKeyDescription("Ctrl:x", "win32")).toBe("Ctrl+X");
    });

    it("follows parseKeyDescription for Option, Alt and Meta", () => {
        // Option is the Alt/Option key on macOS and the Meta (Windows/Super) key elsewhere.
        expect(formatKeyDescription("Option:x", "darwin")).toBe("⌥X");
        expect(formatKeyDescription("Option:x", "win32")).toBe("Win+X");
        expect(formatKeyDescription("Option:x", "linux")).toBe("Super+X");
        // Alt and Meta name the physical key on every platform.
        expect(formatKeyDescription("Alt:x", "darwin")).toBe("⌥X");
        expect(formatKeyDescription("Alt:x", "win32")).toBe("Alt+X");
        expect(formatKeyDescription("Meta:x", "darwin")).toBe("⌘X");
        expect(formatKeyDescription("Meta:x", "win32")).toBe("Win+X");
        expect(formatKeyDescription("Meta:x", "linux")).toBe("Super+X");
        for (const platform of PLATFORMS) {
            for (const keyDescription of ["Option:x", "Alt:x", "Meta:x"]) {
                const label = formatKeyDescription(keyDescription, platform);
                expect(labelMatches(label, keyDescription, platform), `${keyDescription} on ${platform}`).toBe(true);
            }
        }
    });

    it("orders modifiers ⌃⌥⇧⌘ on macOS and Ctrl+Alt+Shift on Windows/Linux, whatever the input order", () => {
        expect(formatKeyDescription("Cmd:Shift:Option:Ctrl:k", "darwin")).toBe("⌃⌥⇧⌘K");
        expect(formatKeyDescription("Shift:Alt:Ctrl:k", "win32")).toBe("Ctrl+Alt+Shift+K");
        expect(formatKeyDescription("Meta:Shift:Ctrl:k", "linux")).toBe("Ctrl+Shift+Super+K");
    });

    it("shows Shift for an upper-case letter, which the matcher requires", () => {
        expect(formatKeyDescription("Cmd:T", "darwin")).toBe("⇧⌘T");
        expect(formatKeyDescription("Cmd:T", "win32")).toBe("Alt+Shift+T");
        for (const platform of PLATFORMS) {
            expect(labelMatches(formatKeyDescription("Cmd:T", platform), "Cmd:T", platform)).toBe(true);
        }
    });

    it("names Space and passes other named keys through", () => {
        expect(formatKeyDescription("Cmd: ", "darwin")).toBe("⌘Space");
        expect(formatKeyDescription("Ctrl:Space", "win32")).toBe("Ctrl+Space");
        expect(formatKeyDescription("Cmd:F2", "win32")).toBe("Alt+F2");
        expect(formatKeyDescription("Shift:Enter", "darwin")).toBe("⇧↩");
        expect(formatKeyDescription("Shift:Enter", "win32")).toBe("Shift+Enter");
    });

    it("defaults to the platform set by setKeyUtilPlatform", () => {
        setKeyUtilPlatform("win32");
        expect(formatKeyDescription("Cmd:t")).toBe("Alt+T");
        setKeyUtilPlatform("darwin");
        expect(formatKeyDescription("Cmd:t")).toBe("⌘T");
    });

    it("an explicit platform does not change the platform the matcher uses", () => {
        setKeyUtilPlatform("win32");
        formatKeyDescription("Cmd:t", "darwin");
        const altT = adaptFromReactOrNativeKeyEvent(new KeyboardEvent("keydown", { key: "t", altKey: true }));
        expect(checkKeyPressed(altT, "Cmd:t")).toBe(true);
    });

    // The labels the hamburger menu used to hard-code. Each names a key
    // combination that does not fire its binding.
    it.each([
        ["Ctrl+T", "Cmd:t", "win32"],
        ["Ctrl+T", "Cmd:t", "linux"],
        ["⌘⇧N", "Ctrl:Shift:n", "darwin"],
        ["⌘P", "Ctrl:p", "darwin"],
    ] as [string, string, Platform][])("the old label %s does not fire %s on %s", (label, keyDescription, platform) => {
        expect(labelMatches(label, keyDescription, platform)).toBe(false);
        expect(formatKeyDescription(keyDescription, platform)).not.toBe(label);
    });
});
