// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
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
