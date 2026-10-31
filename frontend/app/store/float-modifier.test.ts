// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

let mac = false;
let multiWindow = true;
vi.mock("@/util/platformutil", () => ({
    isModKey: (e: { metaKey: boolean; ctrlKey: boolean }) => (mac ? e.metaKey : e.ctrlKey),
}));
vi.mock("@/app/host/host-caps", () => ({ hostHas: () => multiWindow }));

import { installFloatModifier, isFloatClick, noteClick, takeFloatRequest } from "./float-modifier";

const click = (over: Partial<MouseEvent> = {}) => ({ ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...over });

beforeEach(() => {
    mac = false;
    multiWindow = true;
    noteClick(click()); // clear whatever an earlier test left
});

describe("the float modifier", () => {
    it("is Ctrl on Windows and Linux and Cmd on macOS, alone", () => {
        expect(isFloatClick(click({ ctrlKey: true }))).toBe(true);
        expect(isFloatClick(click({ metaKey: true }))).toBe(false);
        expect(isFloatClick(click({ ctrlKey: true, shiftKey: true }))).toBe(false);
        expect(isFloatClick(click({ ctrlKey: true, altKey: true }))).toBe(false);
        mac = true;
        expect(isFloatClick(click({ metaKey: true }))).toBe(true);
        expect(isFloatClick(click({ ctrlKey: true }))).toBe(false);
    });

    it("gives the first pane open after the click a yes, and no open after that", () => {
        noteClick(click({ ctrlKey: true }), 1000);
        expect(takeFloatRequest(1100)).toBe(true);
        expect(takeFloatRequest(1100)).toBe(false);
    });

    it("lapses when the click opened nothing", () => {
        noteClick(click({ ctrlKey: true }), 1000);
        expect(takeFloatRequest(1000 + 601)).toBe(false);
    });

    it("says no for a plain click, and a plain click after a modifier click takes the note back", () => {
        noteClick(click(), 1000);
        expect(takeFloatRequest(1001)).toBe(false);
        noteClick(click({ ctrlKey: true }), 1000);
        noteClick(click(), 1002);
        expect(takeFloatRequest(1003)).toBe(false);
    });

    it("is ignored on a host that has no windows of its own", () => {
        multiWindow = false;
        noteClick(click({ ctrlKey: true }), 1000);
        expect(takeFloatRequest(1001)).toBe(false);
    });
});

describe("installFloatModifier", () => {
    let target: EventTarget;
    beforeEach(() => {
        target = new EventTarget();
        installFloatModifier(target as unknown as Window);
    });
    afterEach(() => noteClick(click()));

    const fire = (type: string, init: Record<string, unknown>) => {
        const ev = new Event(type) as Event & Record<string, unknown>;
        Object.assign(ev, init);
        target.dispatchEvent(ev);
    };
    const withCtrl = { ctrlKey: true, metaKey: false, shiftKey: false, altKey: false };
    const withoutCtrl = { ctrlKey: false, metaKey: false, shiftKey: false, altKey: false };

    it("notes a Ctrl click and a Ctrl double-click", () => {
        fire("click", withCtrl);
        expect(takeFloatRequest()).toBe(true);
        fire("dblclick", withCtrl);
        expect(takeFloatRequest()).toBe(true);
    });

    it("starts over on a plain press or another key, but not on the modifier key itself", () => {
        fire("click", withCtrl);
        fire("keydown", { key: "Control" });
        expect(takeFloatRequest()).toBe(true);
        fire("click", withCtrl);
        fire("keydown", { key: "a" });
        expect(takeFloatRequest()).toBe(false);
        fire("click", withCtrl);
        fire("pointerdown", withoutCtrl);
        expect(takeFloatRequest()).toBe(false);
        fire("click", withCtrl);
        fire("pointerdown", withCtrl);
        expect(takeFloatRequest()).toBe(true);
    });
});
