// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The shell drawer's resize handle: on the drawer's outer top edge, above the header row, so it sits on
 * the composer's bottom border; and dragging it resizes the drawer the way it always did.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({ setBlockMeta: vi.fn() }));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: (...a: unknown[]) => hub.setBlockMeta(...a) }));
vi.mock("@/app/element/tip", () => ({ tip: () => ({}) }));

import { ResizableDetailsDrawer } from "./ResizableDetailsDrawer";

afterEach(() => {
    cleanup();
    hub.setBlockMeta.mockClear();
});

const handle = (root: HTMLElement) => root.querySelector(".agent-composer-details-resize-handle") as HTMLElement;
const box = (root: HTMLElement) => root.querySelector(".agent-composer-details-resizable") as HTMLElement;

function drag(el: HTMLElement, fromY: number, toY: number) {
    fireEvent.pointerDown(el, { clientY: fromY });
    window.dispatchEvent(new MouseEvent("pointermove", { clientY: toY }));
    window.dispatchEvent(new MouseEvent("pointerup", { clientY: toY }));
}

describe("the shell drawer's resize handle on the drawer's outer edge", () => {
    function setup() {
        const { container } = render(() => (
            <div id="drawer">
                <ResizableDetailsDrawer
                    blockId="b1"
                    persistedHeight={200}
                    handleOnEdge
                    header={<div class="agent-shell-info">header row</div>}
                >
                    <div class="shell">shell</div>
                </ResizableDetailsDrawer>
            </div>
        ));
        return container.querySelector("#drawer") as HTMLElement;
    }

    it("comes before the header row and outside the box it resizes, marked as the edge handle", () => {
        const root = setup();
        const kids = Array.from(root.children).map((c) => c.className);
        expect(kids[0]).toContain("agent-composer-details-resize-handle--edge");
        expect(kids[1]).toBe("agent-shell-info");
        expect(kids[2]).toBe("agent-composer-details-resizable");
        expect(box(root).contains(handle(root))).toBe(false);
        expect(root.querySelectorAll(".agent-composer-details-resize-handle")).toHaveLength(1);
    });

    it("drags up to grow, down to shrink, clamps, and saves the height on release", () => {
        const root = setup();
        expect(box(root).style.height).toBe("200px");
        drag(handle(root), 500, 450);
        expect(box(root).style.height).toBe("250px");
        expect(hub.setBlockMeta).toHaveBeenLastCalledWith("b1", { "term:shellheight": 250 });
        drag(handle(root), 500, 520);
        expect(box(root).style.height).toBe("230px");
        drag(handle(root), 500, 5000);
        expect(box(root).style.height).toBe("120px");
        drag(handle(root), 500, -5000);
        expect(box(root).style.height).toBe("600px");
    });

    it("keeps the handle inside the box, above the body, when it is not asked for on the edge", () => {
        const { container } = render(() => (
            <ResizableDetailsDrawer blockId="b2" persistedHeight={200}>
                <div class="shell">shell</div>
            </ResizableDetailsDrawer>
        ));
        const inner = box(container as HTMLElement);
        expect(inner.firstElementChild?.className).toContain("agent-composer-details-resize-handle");
        expect(inner.firstElementChild?.className).not.toContain("--edge");
    });
});
