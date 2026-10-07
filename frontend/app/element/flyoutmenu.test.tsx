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
vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: vi.fn() }));
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
});
