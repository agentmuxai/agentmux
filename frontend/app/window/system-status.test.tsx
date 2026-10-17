// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's grip sits right of the widgets, before the window controls.
 * docs/reports/REPORT_WINDOW_HEADER_GRIP_PLACEMENT_2026_10_17.md
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ chrome: true }));
vi.mock("@/app/host/host-caps", () => ({ hostHas: () => h.chrome }));
vi.mock("@/app/hook/useWindowDrag.platform", () => ({ useWindowDrag: () => ({ dragProps: {} }) }));
vi.mock("./action-widgets", () => ({ ActionWidgets: () => <div class="action-widgets" data-testid="widgets" /> }));
vi.mock("./window-controls.platform", () => ({
    WindowControlsRight: () => <div class="window-action-buttons" data-testid="controls" />,
}));

import { SystemStatus } from "./system-status";

afterEach(() => {
    cleanup();
    h.chrome = true;
});

const order = (root: Element) => [...root.querySelector(".system-status")!.children].map((c) => c.getAttribute("data-testid"));

describe("the window's grip", () => {
    it("sits between the widgets and the window controls", () => {
        const { container } = render(() => <SystemStatus />);
        expect(order(container)).toEqual(["widgets", "window-grip", "controls"]);
    });

    it("is a drag region whose dots take no events", () => {
        const { container } = render(() => <SystemStatus />);
        const grip = container.querySelector("[data-testid=window-grip]")!;
        expect(grip.getAttribute("data-drag-region")).toBe("true");
        expect(grip.getAttribute("aria-hidden")).toBe("true");
        expect(grip.querySelector("svg")).not.toBeNull();
    });

    it("stays last when the host draws no window controls (macOS's hamburger follows the status area)", () => {
        h.chrome = false;
        const { container } = render(() => <SystemStatus />);
        expect(order(container)).toEqual(["widgets", "window-grip"]);
    });
});
