// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The More dropdown and the submenu of one of its parent rows (Messengers) answer to the More button's hover
 * intent together: a pointer that leaves the dropdown for the flyout beside it must not close both, however slowly
 * it crosses.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// jsdom can't lay out a floating panel; only the placing is stubbed.
vi.mock("@floating-ui/dom", () => ({ autoUpdate: vi.fn(() => vi.fn()) }));
vi.mock("@/app/platform/pane-overlay", () => ({ useHiddenTabMarker: () => () => undefined, usePaneOverlay: vi.fn() }));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })),
    assertMenuInPaintableArea: vi.fn(),
}));

import { MoreDropdown } from "./more-dropdown";

const widget = (label: string, extra: object = {}) =>
    ({ label, icon: "window-maximize", blockdef: { meta: { view: "term" } }, ...extra }) as unknown as WidgetConfigType;

const parent = widget("Messengers", { children: ["discord"] });
const kids = { discord: widget("Discord") };

function mountDropdown() {
    const onSubmenuEnter = vi.fn();
    const onSubmenuLeave = vi.fn();
    const regions: ({ getBoundingClientRect(): DOMRect } | null)[] = [];
    render(() => (
        <MoreDropdown
            widgets={() => [{ key: "msgr", widget: parent }]}
            onClose={vi.fn()}
            onItemContextMenu={vi.fn()}
            anchor={() => document.createElement("div")}
            settings={() => ({})}
            wmap={() => kids}
            onSubmenuEnter={onSubmenuEnter}
            onSubmenuLeave={onSubmenuLeave}
            setSubmenuEl={(el) => regions.push(el)}
        />
    ));
    const dropdown = document.querySelector(
        ".action-widget-more-dropdown:not(.action-widget-pinned-flyout)"
    ) as HTMLElement;
    return { dropdown, onSubmenuEnter, onSubmenuLeave, regions };
}

const rect = (left: number, top: number, right: number, bottom: number) =>
    ({ x: left, y: top, left, top, right, bottom, width: right - left, height: bottom - top }) as DOMRect;

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("the More dropdown's parent rows", () => {
    it("opens a row's flyout on hover and treats it as part of the dropdown: entering it tells the More button", () => {
        const { dropdown, onSubmenuEnter, onSubmenuLeave } = mountDropdown();
        const row = dropdown.querySelector(".action-widget-more-item") as HTMLElement;
        fireEvent.mouseEnter(row);
        vi.advanceTimersByTime(150);
        const flyout = document.querySelector(".action-widget-pinned-flyout") as HTMLElement;
        expect(flyout).not.toBeNull();
        flyout.dispatchEvent(new MouseEvent("mouseenter"));
        expect(onSubmenuEnter).toHaveBeenCalled();
        flyout.dispatchEvent(new MouseEvent("mouseleave", { clientX: 300, clientY: 20 }));
        expect(onSubmenuLeave).toHaveBeenCalledOnce();
    });

    it("aims the More button's close intent at the open flyout", () => {
        const { dropdown, regions } = mountDropdown();
        dropdown.getBoundingClientRect = () => rect(100, 0, 200, 80);
        const row = dropdown.querySelector(".action-widget-more-item") as HTMLElement;
        fireEvent.mouseEnter(row);
        vi.advanceTimersByTime(150);
        const flyout = document.querySelector(".action-widget-pinned-flyout") as HTMLElement;
        flyout.getBoundingClientRect = () => rect(208, 10, 320, 60);
        const region = regions.filter(Boolean).at(-1)!;
        const r = region.getBoundingClientRect();
        expect([r.left, r.top, r.right, r.bottom]).toEqual([208, 10, 320, 60]);
        fireEvent.mouseLeave(row, { clientX: 150, clientY: 10 });
        expect(document.querySelector(".action-widget-pinned-flyout")).not.toBeNull();
    });
});
