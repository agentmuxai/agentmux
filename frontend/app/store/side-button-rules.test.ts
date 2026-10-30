// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    DEBOUNCE_MS,
    decideSideButton,
    hostPressActs,
    isRepeat,
    SIDE_BUTTON_BACK,
    SIDE_BUTTON_FORWARD,
    type SideButtonInput,
} from "./side-button-rules";

const press = (over: Partial<SideButtonInput> = {}): SideButtonInput => ({
    button: SIDE_BUTTON_BACK,
    buttons: 0,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
    ...over,
});
const on = { enabled: true, modalOpen: false, menuOpen: false };

describe("decideSideButton", () => {
    it("acts on Back and Forward, and consumes them", () => {
        expect(decideSideButton(press(), on)).toEqual({ consume: true, act: -1 });
        expect(decideSideButton(press({ button: SIDE_BUTTON_FORWARD }), on)).toEqual({ consume: true, act: 1 });
    });

    it("leaves every other button alone", () => {
        for (const button of [0, 1, 2]) {
            expect(decideSideButton(press({ button }), on)).toEqual({ consume: false, act: null });
        }
    });

    it("does nothing, and consumes nothing, when the setting is off", () => {
        expect(decideSideButton(press(), { ...on, enabled: false })).toEqual({ consume: false, act: null });
    });

    it("consumes but does not act with a modifier held (Ctrl + button is the browser pane's page history)", () => {
        for (const mod of ["ctrlKey", "shiftKey", "altKey", "metaKey"] as const) {
            expect(decideSideButton(press({ [mod]: true }), on)).toEqual({ consume: true, act: null });
        }
    });

    it("does not act mid-drag (left button held)", () => {
        expect(decideSideButton(press({ buttons: 1 }), on)).toEqual({ consume: true, act: null });
        expect(decideSideButton(press({ buttons: 2 }), on)).toEqual({ consume: true, act: -1 });
    });

    it("does not act while a modal is open, but still keeps Chromium from navigating", () => {
        expect(decideSideButton(press(), { ...on, modalOpen: true })).toEqual({ consume: true, act: null });
    });

    it("does not act while a context menu is open, but still keeps Chromium from navigating", () => {
        expect(decideSideButton(press(), { ...on, menuOpen: true })).toEqual({ consume: true, act: null });
        expect(decideSideButton(press({ button: SIDE_BUTTON_FORWARD }), { ...on, menuOpen: true })).toEqual({ consume: true, act: null });
    });
});

describe("isRepeat", () => {
    it("treats the same button within the window as one press", () => {
        expect(isRepeat(-1, 1000 + DEBOUNCE_MS - 1, { dir: -1, at: 1000 })).toBe(true);
        expect(isRepeat(-1, 1000 + DEBOUNCE_MS, { dir: -1, at: 1000 })).toBe(false);
    });

    it("lets the other direction through at once, and a first press", () => {
        expect(isRepeat(1, 1010, { dir: -1, at: 1000 })).toBe(false);
        expect(isRepeat(-1, 1000, null)).toBe(false);
    });
});

describe("hostPressActs", () => {

    it("acts on a plain press the host reports", () => {
        expect(hostPressActs({ ctrl: false }, on)).toBe(true);
        expect(hostPressActs({ ctrl: false, blocked: false }, on)).toBe(true);
    });

    it("declines what the page listener declines: a modified press or one during a drag", () => {
        expect(hostPressActs({ ctrl: false, blocked: true }, on)).toBe(false);
    });

    it("leaves Ctrl presses to the host and respects the setting and open modals", () => {
        expect(hostPressActs({ ctrl: true }, on)).toBe(false);
        expect(hostPressActs({ ctrl: false }, { ...on, enabled: false })).toBe(false);
        expect(hostPressActs({ ctrl: false }, { ...on, modalOpen: true })).toBe(false);
        expect(hostPressActs({ ctrl: false }, { ...on, menuOpen: true })).toBe(false);
    });
});
