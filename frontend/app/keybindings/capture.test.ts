// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { captureKey, type CaptureEvent } from "./capture";
import { matchKey, parseKey } from "./keys";

/** A key press as the browser reports it, with what the test does not name left off. */
const press = (key: string, code: string, mods: Partial<CaptureEvent> = {}): CaptureEvent => ({
    key,
    code,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
    ...mods,
});

describe("captureKey", () => {
    it("writes modifiers in a fixed order, then the key", () => {
        expect(captureKey(press("T", "KeyT", { ctrlKey: true, shiftKey: true }))).toEqual({ key: "ctrl+shift+t" });
        expect(captureKey(press("w", "KeyW", { metaKey: true, altKey: true, ctrlKey: true, shiftKey: true }))).toEqual({ key: "ctrl+alt+shift+meta+w" });
    });

    it("takes a letter by what it types, or by its physical key when that is not a Latin letter", () => {
        expect(captureKey(press("o", "KeyS", { ctrlKey: true }))).toEqual({ key: "ctrl+o" });
        // macOS Option+T types a dagger.
        expect(captureKey(press("†", "KeyT", { altKey: true }))).toEqual({ key: "alt+t" });
        expect(captureKey(press("Dead", "KeyE", { altKey: true }))).toEqual({ key: "alt+e" });
    });

    it("takes a digit or punctuation by its physical key, so other layouts keep working", () => {
        // AZERTY: the key that types "&" is Digit1.
        expect(captureKey(press("&", "Digit1", { ctrlKey: true }))).toEqual({ key: "ctrl+1" });
        expect(captureKey(press("!", "Digit1", { shiftKey: true, ctrlKey: true }))).toEqual({ key: "ctrl+shift+1" });
        expect(captureKey(press("]", "BracketRight", { metaKey: true }))).toEqual({ key: "meta+]" });
        expect(captureKey(press("/", "Slash", { ctrlKey: true }))).toEqual({ key: "ctrl+/" });
    });

    it("takes named keys by name, and the rest by physical key", () => {
        expect(captureKey(press("ArrowUp", "ArrowUp", { ctrlKey: true, shiftKey: true }))).toEqual({ key: "ctrl+shift+ArrowUp" });
        expect(captureKey(press("F6", "F6"))).toEqual({ key: "F6" });
        expect(captureKey(press("Enter", "Enter"))).toEqual({ key: "Enter" });
        expect(captureKey(press(" ", "Space", { ctrlKey: true }))).toEqual({ key: "ctrl+code:Space" });
        expect(captureKey(press("5", "Numpad5", { ctrlKey: true }))).toEqual({ key: "ctrl+code:Numpad5" });
        expect(captureKey(press("<", "IntlBackslash", { ctrlKey: true }))).toEqual({ key: "ctrl+code:IntlBackslash" });
    });

    it("is not a shortcut when only a modifier is down, while composing, or held down", () => {
        expect(captureKey(press("Control", "ControlLeft", { ctrlKey: true }))).toEqual({ ignored: "modifier" });
        expect(captureKey(press("Shift", "ShiftLeft", { shiftKey: true }))).toEqual({ ignored: "modifier" });
        expect(captureKey(press("a", "KeyA", { isComposing: true }))).toEqual({ ignored: "composing" });
        expect(captureKey(press("Process", "KeyA"))).toEqual({ ignored: "composing" });
        expect(captureKey(press("t", "KeyT", { ctrlKey: true, repeat: true }))).toEqual({ ignored: "repeat" });
        expect(captureKey(press("Unidentified", ""))).toEqual({ ignored: "unknown" });
    });

    it("is not a shortcut when AltGr types a character, which the dispatcher would never match", () => {
        const altGr = (k: string) => k === "AltGraph";
        // Windows: AltGr+Q types "@" on a German layout and reports Ctrl+Alt.
        expect(captureKey(press("@", "KeyQ", { ctrlKey: true, altKey: true, getModifierState: altGr }))).toEqual({ ignored: "altgr" });
        expect(captureKey(press("�", "KeyE", { ctrlKey: true, altKey: true, getModifierState: altGr }))).toEqual({ ignored: "altgr" });
        // Ctrl+Alt+Q on a layout with no AltGr is a shortcut, and so is a Latin letter typed with AltGr down.
        expect(captureKey(press("q", "KeyQ", { ctrlKey: true, altKey: true }))).toEqual({ key: "ctrl+alt+q" });
        expect(captureKey(press("q", "KeyQ", { ctrlKey: true, altKey: true, getModifierState: altGr }))).toEqual({ key: "ctrl+alt+q" });
    });

    it("writes what the dispatcher matches again, on every platform", () => {
        const presses = [
            press("T", "KeyT", { ctrlKey: true, shiftKey: true }),
            press("†", "KeyT", { altKey: true }),
            press("&", "Digit1", { ctrlKey: true }),
            press("]", "BracketRight", { metaKey: true }),
            press("F6", "F6", { shiftKey: true }),
            press(" ", "Space", { ctrlKey: true }),
            press("5", "Numpad5", { ctrlKey: true }),
        ];
        for (const platform of ["mac", "linux", "other"] as const) {
            for (const e of presses) {
                const c = captureKey(e);
                if (!("key" in c)) throw new Error("not captured");
                expect(matchKey(e, parseKey(c.key, platform))).toBe(true);
            }
        }
    });
});
