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
const reduce = vi.hoisted(() => ({ categories: new Set<string>() }));
vi.mock("@/app/motion/motion", () => ({
    shouldReduceMotion: (c: string) => reduce.categories.has(c),
    scaledMs: (ms: number) => ms,
}));
const computeMenuPosition = vi.hoisted(() => vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })));
vi.mock("@/app/util/menu-position", () => ({ computeMenuPosition }));

import { installFakeAnimate, type FakeAnimate } from "@/app/motion/test-utils";
import { Menu, type MenuEntry } from "./Menu";

let frames: FrameRequestCallback[] = [];
const flushFrames = (): void => {
    const due = frames;
    frames = [];
    for (const cb of due) cb(0);
};

/** Run the menu's first frame and let the position it asked for land. */
const placed = async (): Promise<void> => {
    flushFrames();
    await Promise.resolve();
    await Promise.resolve();
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
        reduce.categories.clear();
    });

    it("is a menu of rows with a divider, a danger tone and an optional header", async () => {
        const { menu } = mount({ header: "alpha is already open." });
        await placed();
        expect(menu.getAttribute("role")).toBe("menu");
        expect(menu.querySelector(".ui-menu-header")?.textContent).toBe("alpha is already open.");
        expect(screen.getAllByRole("menuitem").map((r) => r.textContent)).toEqual(["Rename", "Locked", "Delete"]);
        expect(menu.querySelector(".menu-divider")).not.toBeNull();
        expect(screen.getByTestId("delete").classList.contains("ui-menu-item--danger")).toBe(true);
        expect(screen.getByTestId("locked").getAttribute("aria-disabled")).toBe("true");
    });

    it("is marked operator-only when its pick is the person's approval, and not otherwise", async () => {
        const { menu } = mount({ operatorOnly: true });
        await placed();
        expect(menu.hasAttribute("data-operator-only")).toBe(true);
        cleanup();
        const plain = mount();
        await placed();
        expect(plain.menu.hasAttribute("data-operator-only")).toBe(false);
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

    it("takes focus once placed and is driven from the keyboard, skipping disabled rows and dividers", async () => {
        const { calls, menu } = mount();
        // Not while it is hidden awaiting its first placement (a browser would refuse the focus).
        expect(document.activeElement).not.toBe(menu);
        await placed();
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

    describe("entrance and exit", () => {
        let fake: FakeAnimate;
        beforeEach(() => {
            fake = installFakeAnimate();
        });
        afterEach(() => fake.restore());

        const shellOf = (menu: HTMLElement) => menu.parentElement as HTMLElement;

        it("waits out of sight for its first placement, then fades in", async () => {
            const { menu } = mount();
            expect(shellOf(menu).style.visibility).toBe("hidden");
            expect(fake.calls).toHaveLength(0);
            await placed();
            expect(shellOf(menu).style.visibility).toBe("");
            expect(fake.calls).toHaveLength(1);
            expect(fake.calls[0].el).toBe(menu);
            expect(fake.calls[0].keyframes[0]).toMatchObject({ opacity: 0 });
            expect(fake.calls[0].keyframes[1]).toMatchObject({ opacity: 1 });
        });

        it("fades a copy out when it unmounts, and the copy removes itself", async () => {
            const { menu } = mount();
            await placed();
            fake.finishAll();
            cleanup();
            const copy = document.querySelector<HTMLElement>('body > .anchored-popover[aria-hidden="true"]');
            expect(copy).not.toBeNull();
            expect(copy!.style.pointerEvents).toBe("none");
            expect(copy!.hasAttribute("data-pane-overlay")).toBe(false);
            // A picture only: nothing in it can be focused, and a test id names the real menu alone.
            expect(copy!.hasAttribute("inert")).toBe(true);
            expect(document.querySelectorAll('[data-testid="menu"], [role="menuitem"][data-testid]')).toHaveLength(0);
            expect(menu.isConnected).toBe(false);
            const exiting = fake.calls[fake.calls.length - 1];
            expect(exiting.el).toBe(copy);
            expect(exiting.keyframes[1]).toMatchObject({ opacity: 0 });
            exiting.finish();
            await Promise.resolve();
            expect(copy!.isConnected).toBe(false);
        });

        it("shows and removes at once when motion is off", async () => {
            reduce.categories.add("transition");
            const { menu } = mount();
            await placed();
            expect(shellOf(menu).style.visibility).toBe("");
            cleanup();
            await Promise.resolve();
            expect(fake.calls).toHaveLength(0);
            expect(document.querySelector(".anchored-popover")).toBeNull();
        });
    });
});
