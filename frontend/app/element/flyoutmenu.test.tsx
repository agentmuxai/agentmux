// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * FlyoutMenu closes when a leaf item is clicked, unless the item sets
 * `keepOpen` — the hamburger's Theme and Opacity choices, so the user can
 * try several in a row. An outside click still closes it either way.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

// jsdom can't lay out a floating panel; only positioning is stubbed.
vi.mock("@floating-ui/dom", () => ({ autoUpdate: vi.fn(() => vi.fn()) }));
vi.mock("@/app/platform/pane-overlay", () => ({
    useHiddenTabMarker: () => () => undefined, usePaneOverlay: vi.fn() }));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })),
    assertMenuInPaintableArea: vi.fn(),
}));

import { FlyoutMenu } from "./flyoutmenu";

const menuOpen = () => document.querySelector(".menu") !== null;

function open() {
    fireEvent.click(screen.getByText("Open menu"));
}

describe("FlyoutMenu", () => {
    afterEach(() => cleanup());

    it("closes after a plain item is clicked", () => {
        const onClick = vi.fn();
        render(() => (
            <FlyoutMenu items={[{ label: "Plain", onClick }]}>
                <button type="button">Open menu</button>
            </FlyoutMenu>
        ));
        open();
        fireEvent.click(screen.getByText("Plain"));
        expect(onClick).toHaveBeenCalledTimes(1);
        expect(menuOpen()).toBe(false);
    });

    it("hands its owner a close function that closes it from outside", () => {
        let close: (() => void) | undefined;
        const onOpenChange = vi.fn();
        render(() => (
            <FlyoutMenu items={[{ label: "Plain" }]} onOpenChange={onOpenChange} closeRef={(fn) => (close = fn)}>
                <button type="button">Open menu</button>
            </FlyoutMenu>
        ));
        open();
        expect(menuOpen()).toBe(true);
        close!();
        expect(menuOpen()).toBe(false);
        expect(onOpenChange).toHaveBeenLastCalledWith(false);

        // Closing a closed menu does nothing.
        onOpenChange.mockClear();
        close!();
        expect(onOpenChange).not.toHaveBeenCalled();
    });

    it("stays open after a keepOpen item is clicked, and closes on an outside click", () => {
        const onClick = vi.fn();
        render(() => (
            <FlyoutMenu items={[{ label: "Try me", keepOpen: true, onClick }]}>
                <button type="button">Open menu</button>
            </FlyoutMenu>
        ));
        open();
        fireEvent.click(screen.getByText("Try me"));
        fireEvent.click(screen.getByText("Try me"));
        expect(onClick).toHaveBeenCalledTimes(2);
        expect(menuOpen()).toBe(true);

        fireEvent.mouseDown(document.body);
        expect(menuOpen()).toBe(false);
    });

    it("moves a getter-based checkmark without remounting the row", () => {
        const [picked, setPicked] = createSignal("a");
        const items = ["a", "b"].map((id) => ({
            label: id.toUpperCase(),
            keepOpen: true,
            get checked() {
                return picked() === id;
            },
            onClick: () => setPicked(id),
        }));
        render(() => (
            <FlyoutMenu items={items}>
                <button type="button">Open menu</button>
            </FlyoutMenu>
        ));
        open();
        const rowB = screen.getByText("B").closest(".menu-item")!;
        const checkOf = (row: Element) => row.querySelector(".menu-item-check")!.classList.contains("fa-check");
        expect(checkOf(rowB)).toBe(false);

        fireEvent.click(screen.getByText("B"));
        expect(checkOf(rowB)).toBe(true);
        // Same DOM node: the row was updated in place, not rebuilt.
        expect(screen.getByText("B").closest(".menu-item")).toBe(rowB);
        expect(checkOf(screen.getByText("A").closest(".menu-item")!)).toBe(false);
    });

    it("re-positions when the container it is kept inside resizes, and stops watching when it closes", async () => {
        const observed: Element[] = [];
        const disconnect = vi.fn();
        let fire: () => void = () => {};
        vi.stubGlobal(
            "ResizeObserver",
            class {
                constructor(cb: () => void) {
                    fire = cb;
                }
                observe(el: Element) {
                    observed.push(el);
                }
                disconnect = disconnect;
            }
        );
        const { computeMenuPosition } = await import("@/app/util/menu-position");
        const pane = document.createElement("div");
        render(() => (
            <FlyoutMenu items={[{ label: "Plain" }]} containWithin={() => pane}>
                <button type="button">Open menu</button>
            </FlyoutMenu>
        ));
        open();
        await new Promise((r) => requestAnimationFrame(() => r(null)));
        expect(observed).toEqual([pane]);
        const before = vi.mocked(computeMenuPosition).mock.calls.length;
        fire();
        expect(vi.mocked(computeMenuPosition).mock.calls.length).toBeGreaterThan(before);
        // Closing the menu, not unmounting the component, stops the watching.
        expect(disconnect).not.toHaveBeenCalled();
        fireEvent.mouseDown(document.body);
        expect(menuOpen()).toBe(false);
        expect(disconnect).toHaveBeenCalledTimes(1);
        const afterClose = vi.mocked(computeMenuPosition).mock.calls.length;
        fire();
        expect(vi.mocked(computeMenuPosition).mock.calls.length).toBe(afterClose);
        vi.unstubAllGlobals();
    });
});
