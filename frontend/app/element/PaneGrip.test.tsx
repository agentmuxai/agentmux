// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PaneGrip } from "./PaneGrip";

afterEach(() => cleanup());

// SPEC_DRAG_SPACE_AFFORDANCE_2026_10_13.md §5.2, §5.8.
describe("PaneGrip", () => {
    const mount = (dragDisabled?: boolean) => {
        const onMenu = vi.fn();
        const { container } = render(() => (
            <div onContextMenu={(e) => onMenu(e)}>
                <PaneGrip dragDisabled={dragDisabled} />
            </div>
        ));
        return { grip: container.querySelector(".pane-grip")!, onMenu };
    };

    it("is a named button that offers a menu, drawn as a 4×3 grip", () => {
        const { grip } = mount();
        expect(grip.getAttribute("role")).toBe("button");
        expect(grip.getAttribute("tabindex")).toBe("0");
        expect(grip.getAttribute("aria-label")).toBe("Move pane / pane actions");
        expect(grip.getAttribute("aria-haspopup")).toBe("menu");
        expect(grip.querySelectorAll("svg circle").length).toBe(12);
    });

    it("a click opens the pane menu (the header's own contextmenu)", () => {
        const { grip, onMenu } = mount();
        fireEvent.click(grip);
        expect(onMenu).toHaveBeenCalledTimes(1);
    });

    it("a click that ends a drag (a floating pane moved by its grip) doesn't open the menu", () => {
        const { grip, onMenu } = mount();
        fireEvent.pointerDown(grip, { screenX: 100, screenY: 100 });
        fireEvent.click(grip, { screenX: 180, screenY: 130 });
        expect(onMenu).not.toHaveBeenCalled();
        fireEvent.pointerDown(grip, { screenX: 100, screenY: 100 });
        fireEvent.click(grip, { screenX: 101, screenY: 100 });
        expect(onMenu).toHaveBeenCalledTimes(1);
    });

    it("a double-click on the grip doesn't reach the header (no magnify)", () => {
        const onDbl = vi.fn();
        const { container } = render(() => (
            <div onDblClick={onDbl}>
                <PaneGrip />
            </div>
        ));
        fireEvent.dblClick(container.querySelector(".pane-grip")!);
        expect(onDbl).not.toHaveBeenCalled();
    });

    it("Enter and Space open the pane menu too", () => {
        const { grip, onMenu } = mount();
        fireEvent.keyDown(grip, { key: "Enter" });
        fireEvent.keyDown(grip, { key: " " });
        fireEvent.keyDown(grip, { key: "a" });
        expect(onMenu).toHaveBeenCalledTimes(2);
    });

    it("a pane that can't be dragged (magnified, ephemeral) keeps the menu but drops 'move'", () => {
        const { grip, onMenu } = mount(true);
        expect(grip.classList.contains("pane-grip--no-drag")).toBe(true);
        expect(grip.getAttribute("aria-label")).toBe("Pane actions");
        expect(grip.getAttribute("title")).toBe("Pane actions");
        fireEvent.click(grip);
        expect(onMenu).toHaveBeenCalledTimes(1);
    });
});
