// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * AgentRowPanel: the frame of the panel under a My agents tile. The caret points at the tile's middle, as a
 * share of the panel's width; the pane is scrolled to the panel's bottom when it opens, when it shows
 * something else, and when its opening move ends. jsdom has no layout, so the rects are given.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const revealDetail = vi.hoisted(() => vi.fn());
vi.mock("@/app/element/ui/detail-reveal", () => ({ revealDetail }));

import { AgentRowPanel } from "./AgentRowPanel";

let frames: FrameRequestCallback[] = [];
const flushFrames = (): void => {
    const due = frames;
    frames = [];
    for (const cb of due) cb(0);
};

const box = (left: number, width: number): DOMRect =>
    ({ left, width, right: left + width, top: 0, bottom: 10, height: 10, x: left, y: 0, toJSON: () => ({}) }) as DOMRect;

function mount(tileBox: DOMRect, panelBox: DOMRect, extra: { onBack?: () => void; onClose?: () => void } = {}) {
    const tile = document.createElement("li");
    tile.getBoundingClientRect = () => tileBox;
    const [view, setView] = createSignal("actions");
    render(() => (
        <ul>
            <AgentRowPanel
                title="alpha"
                subtitle={view() === "actions" ? undefined : "Rename"}
                viewKey={view()}
                tile={() => tile}
                onClose={extra.onClose ?? (() => {})}
                onBack={extra.onBack}
            >
                <p>body</p>
            </AgentRowPanel>
        </ul>
    ));
    const panel = screen.getByTestId("agent-row-panel");
    panel.getBoundingClientRect = () => panelBox;
    return { panel, tile, setView };
}

describe("AgentRowPanel", () => {
    beforeEach(() => {
        frames = [];
        revealDetail.mockClear();
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
        vi.stubGlobal("cancelAnimationFrame", () => {});
    });
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it("is a list row in the New Agent panel's frame, titled with the agent", () => {
        const { panel } = mount(box(20, 300), box(0, 340));
        expect(panel.tagName).toBe("LI");
        expect(panel.classList.contains("ui-card-detail")).toBe(true);
        expect(panel.querySelector(".agent-create-panel-title")?.textContent).toBe("alpha");
    });

    it("points the caret at the middle of the tile, as a share of the panel's width", () => {
        // Tile 100..300 has its middle at 200; the panel is 0..400 wide.
        const { panel } = mount(box(100, 200), box(0, 400));
        flushFrames();
        expect(panel.style.getPropertyValue("--ui-card-caret-left")).toBe("50%");
    });

    it("follows a tile that is not under the panel's middle, and stays off the very edge", () => {
        const { panel } = mount(box(0, 100), box(0, 400));
        flushFrames();
        expect(panel.style.getPropertyValue("--ui-card-caret-left")).toBe("12.5%");
        cleanup();
        const far = mount(box(-500, 10), box(0, 400));
        flushFrames();
        expect(far.panel.style.getPropertyValue("--ui-card-caret-left")).toBe("2%");
    });

    it("scrolls to its bottom when it opens, and again when it shows something else", () => {
        const { tile, panel, setView } = mount(box(100, 200), box(0, 400));
        flushFrames();
        expect(revealDetail).toHaveBeenCalledTimes(1);
        expect(revealDetail).toHaveBeenCalledWith(panel, tile);
        setView("rename");
        flushFrames();
        expect(revealDetail).toHaveBeenCalledTimes(2);
        expect(panel.dataset.view).toBe("rename");
        expect(panel.querySelector(".agent-row-panel-subtitle")?.textContent).toContain("Rename");
    });

    it("scrolls again when its opening move ends, as its first measure was mid-move", () => {
        const { panel } = mount(box(100, 200), box(0, 400));
        flushFrames();
        revealDetail.mockClear();
        fireEvent.animationEnd(panel);
        expect(revealDetail).toHaveBeenCalledTimes(1);
    });

    it("has Back only when it is given one, and always a Close", () => {
        const onBack = vi.fn();
        const onClose = vi.fn();
        mount(box(0, 100), box(0, 400), { onBack, onClose });
        fireEvent.click(screen.getByTestId("agent-row-panel-back"));
        fireEvent.click(screen.getByTestId("agent-row-panel-close"));
        expect(onBack).toHaveBeenCalledTimes(1);
        expect(onClose).toHaveBeenCalledTimes(1);
        cleanup();
        mount(box(0, 100), box(0, 400));
        expect(screen.queryByTestId("agent-row-panel-back")).toBeNull();
        expect(screen.getByTestId("agent-row-panel-close")).toBeInTheDocument();
    });
});
