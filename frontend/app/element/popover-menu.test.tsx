// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

// jsdom can't lay out a floating panel; only the placing is stubbed.
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({
        style: { position: "fixed", left: "0px", top: "0px" },
        placement: "bottom-start",
        maxHeight: Infinity,
        maxWidth: Infinity,
    })),
}));

import { PopoverMenu, type PopoverMenuItem } from "./popover-menu";

afterEach(() => cleanup());

const items = (collapsible?: boolean): PopoverMenuItem[] => [
    {
        type: "section",
        label: "Pin Widgets",
        collapsible,
        items: [{ type: "checkbox", label: "Agent", checked: true, keepOpen: true, click: vi.fn() }],
    },
];

const mount = (collapsible?: boolean) =>
    render(() => <PopoverMenu items={items(collapsible)} pos={{ x: 0, y: 0 }} onClose={vi.fn()} />);

describe("PopoverMenu sections", () => {
    it("a non-collapsible section is a plain label: no chevron, and clicking it hides nothing", () => {
        mount(false);
        const container = document.body;
        const header = container.querySelector(".popover-menu-section-header")!;
        expect(header.classList.contains("popover-menu-section-header--static")).toBe(true);
        expect(header.querySelector(".fa-chevron-down, .fa-chevron-right")).toBeNull();
        fireEvent.click(header);
        expect(container.querySelector(".popover-menu-section-items")).not.toBeNull();
        expect(container.textContent).toContain("Agent");
    });

    it("a section collapses on a header click by default", () => {
        mount();
        const container = document.body;
        const header = container.querySelector(".popover-menu-section-header")!;
        expect(header.querySelector(".fa-chevron-down")).not.toBeNull();
        fireEvent.click(header);
        expect(container.querySelector(".popover-menu-section-items")).toBeNull();
    });
});

describe("PopoverMenu icons", () => {
    const rows = (): HTMLElement[] => [...document.body.querySelectorAll<HTMLElement>(".popover-menu-action")];
    const rowOf = (label: string) => rows().find((r) => r.querySelector(".label")?.textContent === label)!;

    it("draws an action row's icon in the leading slot, hidden from assistive tech", () => {
        render(() => (
            <PopoverMenu
                items={[
                    { label: "New Tab", icon: "plus", click: vi.fn() },
                    { label: "Plain", click: vi.fn() },
                ]}
                pos={{ x: 0, y: 0 }}
                onClose={vi.fn()}
            />
        ));
        const icon = rowOf("New Tab").querySelector(".menu-item-icon")!;
        expect(icon.classList.contains("fa-plus")).toBe(true);
        expect(icon.getAttribute("aria-hidden")).toBe("true");
        expect(rowOf("New Tab").querySelector(".menu-item-icon-spacer")).toBeNull();
        // A row with no icon keeps the blank slot, so the labels line up as they did.
        expect(rowOf("Plain").querySelector(".menu-item-icon-spacer")).not.toBeNull();
    });

    it("keeps a checkbox row's check in the leading slot and draws its icon after it, in its own colour", () => {
        render(() => (
            <PopoverMenu
                items={[
                    { type: "checkbox", label: "Terminal", checked: true, icon: "plus", iconColor: "#58c142", click: vi.fn() },
                    { type: "checkbox", label: "Show Icons Only", checked: false, click: vi.fn() },
                ]}
                pos={{ x: 0, y: 0 }}
                onClose={vi.fn()}
            />
        ));
        const [check, icon] = [...rowOf("Terminal").querySelectorAll<HTMLElement>(".menu-item-icon")];
        expect(check.classList.contains("fa-check")).toBe(true);
        expect(icon.classList.contains("fa-plus")).toBe(true);
        expect(icon.getAttribute("aria-hidden")).toBe("true");
        expect(icon.style.getPropertyValue("--menu-icon-color")).toBe("#58c142");
        // A checkbox row with no icon is just its check, as before.
        expect(rowOf("Show Icons Only").querySelectorAll(".menu-item-icon")).toHaveLength(1);
    });

    it("sets no colour variable on an icon that has no colour", () => {
        render(() => <PopoverMenu items={[{ label: "New Tab", icon: "plus", click: vi.fn() }]} pos={{ x: 0, y: 0 }} onClose={vi.fn()} />);
        expect(rowOf("New Tab").querySelector<HTMLElement>(".menu-item-icon")!.style.getPropertyValue("--menu-icon-color")).toBe("");
    });
});

describe("PopoverMenu chrome zoom", () => {
    it("caps its height at the room the window leaves and scrolls inside it (a long menu at 150% zoom)", async () => {
        const { computeMenuPosition } = await import("@/app/util/menu-position");
        vi.mocked(computeMenuPosition).mockResolvedValueOnce({
            style: { position: "fixed", left: "0px", top: "0px" },
            placement: "bottom-start",
            maxHeight: 600,
            maxWidth: 800,
        });
        mount();
        await new Promise((r) => requestAnimationFrame(() => r(null)));
        await new Promise((r) => setTimeout(r, 0));
        const body = document.body.querySelector<HTMLElement>(".popover-menu")!;
        expect(body.style.maxHeight).toBe("calc(600px / var(--zoomfactor, 1))");
        expect(body.style.overflowY).toBe("auto");
    });

    it("is drawn in AnchoredPopover's zoomed body, so it scales with the chrome like the button that opened it", () => {
        mount();
        const body = document.body.querySelector(".popover-menu")!;
        expect(body.classList.contains("anchored-popover-body--chrome-zoom")).toBe(true);
        expect(body.parentElement!.classList.contains("anchored-popover")).toBe(true);
    });
});
