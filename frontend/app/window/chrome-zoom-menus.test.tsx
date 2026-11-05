// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The title bar's More dropdown and a parent widget's flyout scale with chrome zoom (task #23943): each is drawn in
 * AnchoredPopover's zoomed body, not in a portal of its own that keeps its natural size while the bar grows.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

// jsdom can't lay out a floating panel; only the placing is stubbed.
vi.mock("@floating-ui/dom", () => ({ autoUpdate: vi.fn(() => vi.fn()) }));
vi.mock("@/app/platform/pane-overlay", () => ({ useHiddenTabMarker: () => () => undefined, usePaneOverlay: vi.fn() }));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })),
    assertMenuInPaintableArea: vi.fn(),
}));

import { MoreDropdown } from "./more-dropdown";
import { PinnedWidgetFlyout } from "./pinned-widget-flyout";

afterEach(() => cleanup());

const widget = (label: string) => ({ label, icon: "window-maximize", blockdef: { meta: { view: "term" } } }) as unknown as WidgetConfigType;
const anchor = () => document.createElement("div");

function zoomed(selector: string) {
    const body = document.body.querySelector(selector)!;
    expect(body.classList.contains("anchored-popover-body--chrome-zoom")).toBe(true);
    expect(body.parentElement!.classList.contains("anchored-popover")).toBe(true);
}

describe("title bar menus and chrome zoom", () => {
    it("draws the More dropdown in the zoomed body", () => {
        render(() => (
            <MoreDropdown
                widgets={() => [{ key: "a", widget: widget("A") }]}
                onClose={vi.fn()}
                onItemContextMenu={vi.fn()}
                anchor={anchor}
                settings={() => ({})}
                wmap={() => ({})}
            />
        ));
        zoomed(".action-widget-more-dropdown");
    });

    it("draws a parent widget's flyout in the zoomed body", () => {
        render(() => (
            <PinnedWidgetFlyout
                widget={widget("Parent")}
                wmap={() => ({})}
                settings={() => ({})}
                onClose={vi.fn()}
                onItemContextMenu={vi.fn()}
                anchor={anchor}
            />
        ));
        zoomed(".action-widget-pinned-flyout");
    });
});
