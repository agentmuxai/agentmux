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

    it("draws a checkbox and a radio row with their own roles, aria-checked and a check while checked", async () => {
        mount({
            items: [
                { label: "Fail over", type: "checkbox", checked: true, testId: "on" },
                { label: "Not this", type: "checkbox", checked: false, testId: "off" },
                { label: "Light", type: "radio", checked: true, testId: "radio" },
                { label: "Plain", testId: "plain" },
            ],
        });
        await placed();
        expect(screen.getByTestId("on").getAttribute("role")).toBe("menuitemcheckbox");
        expect(screen.getByTestId("on").getAttribute("aria-checked")).toBe("true");
        expect(screen.getByTestId("on").querySelector(".menu-item-check")?.classList.contains("fa-check")).toBe(true);
        expect(screen.getByTestId("off").getAttribute("aria-checked")).toBe("false");
        expect(screen.getByTestId("off").querySelector(".menu-item-check")?.classList.contains("fa-check")).toBe(false);
        expect(screen.getByTestId("radio").getAttribute("role")).toBe("menuitemradio");
        expect(screen.getByTestId("plain").getAttribute("role")).toBe("menuitem");
        expect(screen.getByTestId("plain").hasAttribute("aria-checked")).toBe(false);
    });

    it("picks a checkbox row like any other", () => {
        const onSelect = vi.fn();
        mount({ items: [{ label: "Fail over", type: "checkbox", checked: false, testId: "box", onSelect }] });
        fireEvent.click(screen.getByTestId("box"));
        expect(onSelect).toHaveBeenCalledTimes(1);
    });

    it("keeps the leading slot on every row when asked to, so labels line up, and not otherwise", async () => {
        const rows: MenuEntry[] = [
            { label: "Copy", icon: "copy", testId: "with" },
            { label: "Other", testId: "without" },
        ];
        mount({ items: rows });
        await placed();
        expect(screen.getByTestId("without").querySelector(".menu-item-icon")).toBeNull();
        cleanup();
        mount({ items: rows, alignIcons: true });
        await placed();
        expect(screen.getByTestId("with").querySelector(".menu-item-icon")).not.toBeNull();
        expect(screen.getByTestId("without").querySelector(".menu-item-icon")).not.toBeNull();
    });

    it("takes a regular-set icon name too", async () => {
        mount({ items: [{ label: "Maximise", icon: "regular@window-maximize", testId: "max" }] });
        await placed();
        expect(screen.getByTestId("max").querySelector("i")?.className).toContain("fa-window-maximize");
    });

    it("draws a swatch in place of an icon and a muted sublabel after the label", async () => {
        mount({ items: [{ label: "Red", swatchColor: "#ff0000", sublabel: "the default", testId: "red" }] });
        await placed();
        const row = screen.getByTestId("red");
        expect((row.querySelector(".menu-item-swatch") as HTMLElement).style.backgroundColor).toBe("rgb(255, 0, 0)");
        expect(row.querySelector(".menu-item-sublabel")?.textContent).toBe("the default");
    });

    it("draws a swatch beside a check, for a colour that is the current choice, and a swatch alone when it is not a choice", async () => {
        mount({
            items: [
                { label: "Red", type: "radio", checked: true, swatchColor: "#f00", testId: "on" },
                { label: "Blue", type: "radio", checked: false, swatchColor: "#00f", testId: "off" },
                { label: "Plain swatch", swatchColor: "#0f0", testId: "bare" },
            ],
        });
        await placed();
        const on = screen.getByTestId("on");
        expect(on.querySelector(".menu-item-check")?.classList.contains("fa-check")).toBe(true);
        expect(on.querySelector(".menu-item-swatch")).not.toBeNull();
        const off = screen.getByTestId("off");
        expect(off.querySelector(".menu-item-check")?.classList.contains("fa-check")).toBe(false);
        expect(off.querySelector(".menu-item-swatch")).not.toBeNull();
        const bare = screen.getByTestId("bare");
        expect(bare.querySelector(".menu-item-check")).toBeNull();
        expect(bare.querySelector(".menu-item-swatch")).not.toBeNull();
    });

    describe("submenus", () => {
        const tree = (calls: string[]): MenuEntry[] => [
            { label: "Pane", testId: "pane", onSelect: () => calls.push("pane") },
            {
                label: "Colour",
                testId: "colour",
                submenu: [
                    { label: "Red", testId: "red", onSelect: () => calls.push("red") },
                    {
                        label: "More",
                        testId: "more",
                        submenu: [{ label: "Teal", testId: "teal", onSelect: () => calls.push("teal") }],
                    },
                ],
            },
        ];
        // A level that has closed leaves a picture of itself fading out (aria-hidden); it is not a menu.
        const menus = () => document.querySelectorAll('.anchored-popover:not([aria-hidden="true"]) [role="menu"]').length;

        it("marks a submenu row as a popup, opens it after a short hover, and closes it when another row is entered", () => {
            vi.useFakeTimers();
            try {
                mount({ items: tree([]) });
                const row = screen.getByTestId("colour");
                expect(row.getAttribute("aria-haspopup")).toBe("menu");
                expect(row.getAttribute("aria-expanded")).toBe("false");
                fireEvent.mouseEnter(row);
                expect(menus()).toBe(1);
                vi.advanceTimersByTime(150);
                expect(menus()).toBe(2);
                expect(row.getAttribute("aria-expanded")).toBe("true");
                expect(screen.getByTestId("red")).toBeTruthy();
                fireEvent.mouseEnter(screen.getByTestId("pane"));
                expect(menus()).toBe(1);
                expect(row.getAttribute("aria-expanded")).toBe("false");
            } finally {
                vi.useRealTimers();
            }
        });

        it("hangs the submenu to the right of its row", () => {
            const { menu } = mount({ items: tree([]) });
            fireEvent.click(screen.getByTestId("colour"));
            flushFrames();
            const placements = computeMenuPosition.mock.calls.map((c) => (c as unknown[])[0] as { placement: string });
            expect(placements.map((p) => p.placement)).toContain("right-start");
            expect(menu.isConnected).toBe(true);
        });

        it("opens from the keyboard with Right or Enter, takes the keys, and Left goes back to the level above", async () => {
            const { menu } = mount({ items: tree([]) });
            await placed();
            fireEvent.keyDown(menu, { key: "ArrowDown" });
            fireEvent.keyDown(menu, { key: "ArrowDown" });
            fireEvent.keyDown(menu, { key: "ArrowRight" });
            await placed();
            expect(menus()).toBe(2);
            const sub = screen.getByTestId("red").closest('[role="menu"]') as HTMLElement;
            expect(document.activeElement).toBe(sub);
            expect(screen.getByTestId("red").classList.contains("active")).toBe(true);
            fireEvent.keyDown(sub, { key: "ArrowLeft" });
            await Promise.resolve();
            expect(menus()).toBe(1);
            expect(document.activeElement).toBe(menu);
        });

        it("closes only the innermost level on Esc, then the menu", async () => {
            const { menu, onClose } = mount({ items: tree([]) });
            await placed();
            fireEvent.keyDown(menu, { key: "ArrowDown" });
            fireEvent.keyDown(menu, { key: "ArrowDown" });
            fireEvent.keyDown(menu, { key: "Enter" });
            await placed();
            expect(menus()).toBe(2);
            fireEvent.keyDown(window, { key: "Escape" });
            expect(menus()).toBe(1);
            expect(onClose).not.toHaveBeenCalled();
            fireEvent.keyDown(window, { key: "Escape" });
            expect(onClose).toHaveBeenCalledTimes(1);
        });

        it("gives the keys back to the level above when a keyboard-focused submenu closes, by Esc or by another row", async () => {
            const { menu } = mount({ items: tree([]) });
            await placed();
            const openFromKeyboard = async () => {
                fireEvent.keyDown(menu, { key: "End" }); // the Colour row
                fireEvent.keyDown(menu, { key: "ArrowRight" });
                await placed();
                expect(document.activeElement).toBe(screen.getByTestId("red").closest('[role="menu"]'));
            };
            await openFromKeyboard();
            fireEvent.keyDown(window, { key: "Escape" });
            await Promise.resolve();
            expect(menus()).toBe(1);
            expect(document.activeElement).toBe(menu);
            // The arrows work again.
            fireEvent.keyDown(menu, { key: "ArrowUp" });
            expect(screen.getByTestId("pane").classList.contains("active")).toBe(true);
            await openFromKeyboard();
            fireEvent.mouseEnter(screen.getByTestId("pane"));
            await Promise.resolve();
            expect(menus()).toBe(1);
            expect(document.activeElement).toBe(menu);
        });

        it("moves the keys into a submenu that hover already opened when Right or Enter asks", async () => {
            vi.useFakeTimers();
            try {
                const { menu } = mount({ items: tree([]) });
                fireEvent.mouseEnter(screen.getByTestId("colour"));
                vi.advanceTimersByTime(150);
                flushFrames();
                expect(menus()).toBe(2);
                const sub = screen.getByTestId("red").closest('[role="menu"]') as HTMLElement;
                await Promise.resolve();
                await Promise.resolve();
                expect(document.activeElement).not.toBe(sub);
                expect(document.activeElement).toBe(menu);
                fireEvent.keyDown(menu, { key: "ArrowDown" });
                fireEvent.keyDown(menu, { key: "ArrowDown" });
                fireEvent.keyDown(menu, { key: "ArrowRight" });
                expect(document.activeElement).toBe(sub);
                expect(screen.getByTestId("red").classList.contains("active")).toBe(true);
            } finally {
                vi.useRealTimers();
            }
        });

        it("builds a new submenu for the row it moves to, not a stale one for the row it left", () => {
            mount({
                items: [
                    { label: "A", testId: "a", submenu: [{ label: "A1", testId: "a1" }] },
                    { label: "B", testId: "b", submenu: [{ label: "B1", testId: "b1" }] },
                ],
            });
            fireEvent.click(screen.getByTestId("a"));
            expect(screen.queryByTestId("a1")).not.toBeNull();
            fireEvent.click(screen.getByTestId("b"));
            expect(screen.queryByTestId("a1")).toBeNull();
            expect(screen.queryByTestId("b1")).not.toBeNull();
            expect(menus()).toBe(2);
        });

        it("keeps the menu open for a press in any level below it, and closes it for a press outside", () => {
            const { onClose } = mount({ items: tree([]) });
            fireEvent.click(screen.getByTestId("colour"));
            fireEvent.click(screen.getByTestId("more"));
            expect(menus()).toBe(3);
            fireEvent.mouseDown(screen.getByTestId("teal"));
            fireEvent.mouseDown(screen.getByTestId("red"));
            expect(onClose).not.toHaveBeenCalled();
            fireEvent.mouseDown(document.body);
            expect(onClose).toHaveBeenCalled();
        });

        it("closes the whole menu on a pick in a submenu, then runs it; a click on the submenu row picks nothing", () => {
            const { calls, onClose } = mount({ items: tree([]) });
            fireEvent.click(screen.getByTestId("colour"));
            expect(onClose).not.toHaveBeenCalled();
            const picks: string[] = [];
            cleanup();
            const m = mount({ items: tree(picks) });
            fireEvent.click(screen.getByTestId("colour"));
            fireEvent.click(screen.getByTestId("red"));
            expect(picks).toEqual(["red"]);
            expect(m.onClose).toHaveBeenCalledTimes(1);
            expect(calls).toEqual([]);
        });
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
