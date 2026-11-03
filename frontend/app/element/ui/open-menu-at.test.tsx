// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * openMenuAt draws the kit Menu for a right-click: the same items as the JS context menu, one menu at a time,
 * downward from the pointer, and focus handed back to what had it.
 */

import { cleanup, fireEvent } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn((_ref: unknown, _el: unknown, update: () => void) => {
        update();
        return vi.fn();
    }),
}));
vi.mock("@/app/platform/pane-overlay", () => ({
    useHiddenTabMarker: () => () => undefined,
    usePaneOverlay: vi.fn(),
}));
vi.mock("@/app/motion/motion", () => ({ shouldReduceMotion: () => true, scaledMs: (ms: number) => ms }));
const computeMenuPosition = vi.hoisted(() => vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })));
vi.mock("@/app/util/menu-position", () => ({ computeMenuPosition }));

import { closeMenuAt, menuEntriesFrom, openMenuAt } from "./open-menu-at";

let frames: FrameRequestCallback[] = [];
const rows = () => [...document.querySelectorAll<HTMLElement>('[role="menu"] > [role^="menuitem"]')];

describe("menuEntriesFrom", () => {
    it("maps a ContextMenuItem list: separators, hidden and disabled rows, checks, icons and picks", () => {
        const click = vi.fn();
        const entries = menuEntriesFrom([
            { label: "Copy", icon: "copy", click },
            { type: "separator" },
            { label: "Hidden", visible: false },
            { label: "Off", enabled: false },
            { label: "Fail over", type: "checkbox", checked: true },
            { label: "Light", type: "radio", checked: false },
        ]);
        expect(entries.map((e) => [e.label, !!e.divider, !!e.disabled, e.type, e.checked, e.icon])).toEqual([
            ["Copy", false, false, undefined, undefined, "copy"],
            ["", true, false, undefined, undefined, undefined],
            ["Off", false, true, undefined, undefined, undefined],
            ["Fail over", false, false, "checkbox", true, undefined],
            ["Light", false, false, "radio", false, undefined],
        ]);
        entries[0].onSelect?.();
        expect(click).toHaveBeenCalledTimes(1);
    });
});

describe("openMenuAt", () => {
    beforeEach(() => {
        frames = [];
        computeMenuPosition.mockClear();
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
        vi.stubGlobal("cancelAnimationFrame", () => {});
    });
    afterEach(() => {
        closeMenuAt();
        cleanup();
        vi.unstubAllGlobals();
        document.body.innerHTML = "";
    });

    const event = (x = 40, y = 90) => ({ stopPropagation: vi.fn(), clientX: x, clientY: y });

    it("stops the event, shows the rows, and hangs the menu downward from the pointer", () => {
        const ev = event();
        openMenuAt([{ label: "Copy" }, { label: "Copy All" }], ev);
        expect(ev.stopPropagation).toHaveBeenCalled();
        expect(rows().map((r) => r.textContent)).toEqual(["Copy", "Copy All"]);
        for (const cb of frames.splice(0)) cb(0);
        const request = (computeMenuPosition.mock.calls[0] as unknown[])[0] as { anchor: DOMRect; placement: string };
        expect(request.placement).toBe("bottom-start");
        expect([request.anchor.x, request.anchor.y, request.anchor.width]).toEqual([40, 90, 0]);
    });

    it("shows nothing for an empty or all-hidden list", () => {
        openMenuAt([], event());
        openMenuAt([{ label: "x", visible: false }], event());
        expect(document.querySelector('[role="menu"]')).toBeNull();
    });

    it("closes the menu that is open when another opens", () => {
        openMenuAt([{ label: "First" }], event());
        openMenuAt([{ label: "Second" }], event());
        expect(rows().map((r) => r.textContent)).toEqual(["Second"]);
    });

    it("closes on a pick and runs it; a disabled row does nothing", () => {
        const click = vi.fn();
        const off = vi.fn();
        openMenuAt([{ label: "Off", enabled: false, click: off }, { label: "Copy", click }], event());
        fireEvent.click(rows()[0]);
        expect(off).not.toHaveBeenCalled();
        fireEvent.click(rows()[1]);
        expect(click).toHaveBeenCalledTimes(1);
        expect(document.querySelector('[role="menu"]')).toBeNull();
    });

    it("closes on an outside press and on Esc", () => {
        openMenuAt([{ label: "Copy" }], event());
        fireEvent.mouseDown(document.body);
        expect(document.querySelector('[role="menu"]')).toBeNull();
        openMenuAt([{ label: "Copy" }], event());
        fireEvent.keyDown(window, { key: "Escape" });
        expect(document.querySelector('[role="menu"]')).toBeNull();
    });

    it("gives focus back to what had it when the menu closes without a pick", async () => {
        const field = document.createElement("input");
        document.body.appendChild(field);
        field.focus();
        openMenuAt([{ label: "Copy" }], event());
        (document.querySelector('[role="menu"]') as HTMLElement).focus();
        fireEvent.mouseDown(document.body);
        (document.activeElement as HTMLElement | null)?.blur();
        await Promise.resolve();
        expect(document.activeElement).toBe(field);
    });

    it("leaves focus where a pick moved it", async () => {
        const field = document.createElement("input");
        const other = document.createElement("input");
        document.body.append(field, other);
        field.focus();
        openMenuAt([{ label: "Go", click: () => other.focus() }], event());
        fireEvent.click(rows()[0]);
        await Promise.resolve();
        expect(document.activeElement).toBe(other);
    });
});
