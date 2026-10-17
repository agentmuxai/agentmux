// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Click zoom on the image viewer: a click zooms in a step, a drag doesn't,
// the zoom-out modifier zooms out, double-click does nothing, and the cursor
// follows. docs/specs/SPEC_EDITOR_TREE_KEYBOARD_ICONS_AND_CLICK_ZOOM_2026_10_16.md §7.

import { setPlatform } from "@/util/platformutil";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ImageViewer } from "./image-viewer";
import { clickSteps, type ZoomBox } from "./image-zoom";

vi.mock("../file-bytes", () => ({
    createFileBytes: () => {
        const [error, setError] = createSignal("");
        return { url: () => "blob:image", error, setError };
    },
}));

// A 2000×1000 image fitted into an 800×600 view: 800×400 on screen.
const BOX: ZoomBox = {
    fitWidth: 800,
    fitHeight: 400,
    viewWidth: 800,
    viewHeight: 600,
    naturalWidth: 2000,
    naturalHeight: 1000,
};
const STEPS = clickSteps(BOX);

function size(el: Element, props: Record<string, number>) {
    for (const [k, v] of Object.entries(props)) Object.defineProperty(el, k, { configurable: true, value: v });
}

function mount() {
    const { container } = render(() => (
        <ImageViewer
            path="C:/pics/a.png"
            mime="image/png"
            revision={0}
            bar={{ viewer: "image", options: ["image"], onReopenWith: () => {} }}
        />
    ));
    const content = container.querySelector<HTMLElement>(".image-viewer-content")!;
    const img = container.querySelector<HTMLImageElement>("img")!;
    size(content, { offsetWidth: BOX.viewWidth, offsetHeight: BOX.viewHeight });
    content.getBoundingClientRect = () =>
        ({ left: 0, top: 0, width: BOX.viewWidth, height: BOX.viewHeight }) as DOMRect;
    size(img, {
        offsetWidth: BOX.fitWidth,
        offsetHeight: BOX.fitHeight,
        naturalWidth: BOX.naturalWidth,
        naturalHeight: BOX.naturalHeight,
    });
    fireEvent.load(img);
    return img;
}

const scaleOf = (img: HTMLElement) => Number(/scale\(([^)]+)\)/.exec(img.style.transform)![1]);

// A press and release at the view's centre, moved `move` px in between.
function click(img: HTMLElement, mods: { ctrlKey?: boolean; altKey?: boolean } = {}, move = 0) {
    fireEvent.pointerDown(img, { button: 0, buttons: 1, pointerId: 1, clientX: 400, clientY: 300, ...mods });
    if (move) fireEvent.pointerMove(img, { buttons: 1, pointerId: 1, clientX: 400 + move, clientY: 300, ...mods });
    fireEvent.pointerUp(img, { button: 0, buttons: 0, pointerId: 1, clientX: 400 + move, clientY: 300, ...mods });
}

describe("the image viewer's click zoom", () => {
    beforeAll(() => {
        Element.prototype.setPointerCapture ??= () => {};
    });
    beforeEach(() => setPlatform("win32"));
    afterEach(() => cleanup());

    it("zooms in a step on a click", () => {
        const img = mount();
        expect(scaleOf(img)).toBe(1);
        click(img);
        expect(scaleOf(img)).toBeCloseTo(STEPS[1]);
        click(img);
        expect(scaleOf(img)).toBeCloseTo(STEPS[2]);
    });

    it("doesn't zoom on a press that moves 4 px or more", () => {
        const img = mount();
        click(img, {}, 5);
        expect(scaleOf(img)).toBe(1);
        click(img);
        click(img, {}, 5);
        expect(scaleOf(img)).toBeCloseTo(STEPS[1]);
        // Under 4 px is still a click.
        click(img, {}, 3);
        expect(scaleOf(img)).toBeCloseTo(STEPS[2]);
    });

    it("zooms out a step on Ctrl+click or Alt+click, and stops at fit", () => {
        const img = mount();
        click(img);
        click(img);
        click(img, { ctrlKey: true });
        expect(scaleOf(img)).toBeCloseTo(STEPS[1]);
        click(img, { altKey: true });
        expect(scaleOf(img)).toBe(1);
        click(img, { altKey: true });
        expect(scaleOf(img)).toBe(1);
    });

    it("on macOS zooms out on ⌥+click and leaves Ctrl+click to the context menu", () => {
        setPlatform("darwin");
        const img = mount();
        click(img);
        click(img, { ctrlKey: true });
        expect(scaleOf(img)).toBeCloseTo(STEPS[1]);
        click(img, { altKey: true });
        expect(scaleOf(img)).toBe(1);
    });

    it("no longer toggles fit and actual size on a double-click", () => {
        const img = mount();
        fireEvent.dblClick(img, { clientX: 400, clientY: 300 });
        expect(scaleOf(img)).toBe(1);
    });

    it("shows zoom-out while the modifier is held, and grab at the deepest step", () => {
        const img = mount();
        fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
        expect(img).toHaveClass("image-viewer-zoom-out");
        fireEvent.keyUp(window, { key: "Control", ctrlKey: false });
        expect(img).not.toHaveClass("image-viewer-zoom-out");
        for (let k = 0; k < 5; k++) click(img);
        expect(scaleOf(img)).toBeCloseTo(STEPS[5]);
        expect(img).toHaveClass("image-viewer-deepest");
        click(img);
        expect(scaleOf(img)).toBeCloseTo(STEPS[5]);
    });
});
