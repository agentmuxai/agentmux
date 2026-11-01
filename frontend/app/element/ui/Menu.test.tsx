// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Menu is the one anchored menu: it opens upward from an element, a rect or a point, closes on a pick,
 * an outside press or Esc, and is driven from the keyboard. Placing is AnchoredPopover's (and the
 * geometry was measured there), so the position call is mocked and what it is asked for is pinned.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
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
const computeMenuPosition = vi.hoisted(() => vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })));
vi.mock("@/app/util/menu-position", () => ({ computeMenuPosition }));

import { Menu, type MenuEntry } from "./Menu";

let frames: FrameRequestCallback[] = [];
const flushFrames = (): void => {
    const due = frames;
    frames = [];
    for (const cb of due) cb(0);
};

const items = (calls: string[]): MenuEntry[] => [
    { label: "Rename", icon: "pen", testId: "rename", onSelect: () => calls.push("rename") },
    { label: "Locked", disabled: true, testId: "locked", onSelect: () => calls.push("locked") },
    { label: "", divider: true },
    { label: "Delete", icon: "trash-can", tone: "danger", testId: "delete", onSelect: () => calls.push("delete") },
];

function mount(over: Partial<Parameters<typeof Menu>[0]> = {}) {
    const calls: string[] = [];
    const anchor = document.createElement("div");
    document.body.appendChild(anchor);
    const onClose = vi.fn(() => calls.push("close"));
    render(() => <Menu anchor={anchor} items={items(calls)} onClose={onClose} testId="menu" {...over} />);
    return { calls, onClose, anchor, menu: screen.getByTestId("menu") };
}

describe("Menu", () => {
    beforeEach(() => {
        frames = [];
        computeMenuPosition.mockClear();
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
        vi.stubGlobal("cancelAnimationFrame", () => {});
    });
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        document.body.innerHTML = "";
    });

    it("is a menu of rows with a divider, a danger tone and an optional header", () => {
        const { menu } = mount({ header: "alpha is already open." });
        expect(menu.getAttribute("role")).toBe("menu");
        expect(menu.querySelector(".ui-menu-header")?.textContent).toBe("alpha is already open.");
        expect(screen.getAllByRole("menuitem").map((r) => r.textContent)).toEqual(["Rename", "Locked", "Delete"]);
        expect(menu.querySelector(".menu-divider")).not.toBeNull();
        expect(screen.getByTestId("delete").classList.contains("ui-menu-item--danger")).toBe(true);
        expect(screen.getByTestId("locked").getAttribute("aria-disabled")).toBe("true");
    });

    it("opens upward by default, and takes the placement it is given", () => {
        mount();
        flushFrames();
        expect((computeMenuPosition.mock.calls[0] as unknown[])[0]).toMatchObject({ placement: "top-start" });
        cleanup();
        computeMenuPosition.mockClear();
        mount({ placement: "bottom-end" });
        flushFrames();
        expect((computeMenuPosition.mock.calls[0] as unknown[])[0]).toMatchObject({ placement: "bottom-end" });
    });

    it("hangs from a point: a right-click gives a rect of no size at the pointer", () => {
        mount({ anchor: { x: 40, y: 90 } });
        flushFrames();
        const request = (computeMenuPosition.mock.calls[0] as unknown[])[0] as { anchor: DOMRect };
        expect(request.anchor.x).toBe(40);
        expect(request.anchor.y).toBe(90);
        expect(request.anchor.width).toBe(0);
    });

    it("closes, then runs the pick; a disabled row does nothing", () => {
        const { calls } = mount();
        fireEvent.click(screen.getByTestId("locked"));
        expect(calls).toEqual([]);
        fireEvent.click(screen.getByTestId("rename"));
        expect(calls).toEqual(["close", "rename"]);
    });

    it("closes on an outside press and on Esc, not on a press inside", () => {
        const { onClose, menu } = mount();
        fireEvent.mouseDown(menu);
        expect(onClose).not.toHaveBeenCalled();
        fireEvent.mouseDown(document.body);
        expect(onClose).toHaveBeenCalledTimes(1);
        fireEvent.keyDown(window, { key: "Escape" });
        expect(onClose).toHaveBeenCalledTimes(2);
    });

    it("does not close on a press on its own anchor, which owns its toggle", () => {
        const { onClose, anchor } = mount();
        fireEvent.mouseDown(anchor);
        expect(onClose).not.toHaveBeenCalled();
    });

    it("takes focus and is driven from the keyboard, skipping disabled rows and dividers", () => {
        const { calls, menu } = mount();
        expect(document.activeElement).toBe(menu);
        fireEvent.keyDown(menu, { key: "ArrowDown" });
        expect(screen.getByTestId("rename").classList.contains("active")).toBe(true);
        fireEvent.keyDown(menu, { key: "ArrowDown" });
        expect(screen.getByTestId("delete").classList.contains("active")).toBe(true);
        fireEvent.keyDown(menu, { key: "ArrowDown" });
        expect(screen.getByTestId("rename").classList.contains("active")).toBe(true);
        fireEvent.keyDown(menu, { key: "ArrowUp" });
        expect(screen.getByTestId("delete").classList.contains("active")).toBe(true);
        fireEvent.keyDown(menu, { key: "Enter" });
        expect(calls).toEqual(["close", "delete"]);
    });

    it("Enter with nothing highlighted picks nothing", () => {
        const { calls, menu } = mount();
        fireEvent.keyDown(menu, { key: "Enter" });
        expect(calls).toEqual([]);
    });
});
