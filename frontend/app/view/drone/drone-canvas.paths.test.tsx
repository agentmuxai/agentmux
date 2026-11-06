// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The drone canvas, one behaviour at a time: the wheel and button zoom, fit-to-view, dragging a
 * node, panning, dropping a chip, wiring one port to another, the keys, and what a press or an
 * unmount leaves behind. The existing drone tests cover the pane zoom and the chip-to-canvas
 * type check; these cover the numbers and the edges of each gesture, so the canvas can be
 * split into smaller functions without changing any of them.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListDronesCommand: () => Promise.resolve([]),
        ListDroneRunsCommand: () => Promise.resolve([]),
        ListBundlesCommand: () => Promise.resolve([]),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { session } from "@/app/drag/drag-session";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { DroneViewModel } from "./drone-model";
import { DroneView } from "./drone-view";

// The canvas sits at (50, 20) and is 800 x 600, so a client point and a canvas point differ.
const RECT = { left: 50, top: 20, width: 800, height: 600 };
let rect = { ...RECT };

const observers: Array<{ cb: () => void; observe: ReturnType<typeof vi.fn>; disconnect: ReturnType<typeof vi.fn> }> =
    [];
vi.stubGlobal(
    "ResizeObserver",
    class {
        disconnect = vi.fn();
        observe = vi.fn();
        constructor(cb: () => void) {
            observers.push({ cb, observe: this.observe, disconnect: this.disconnect });
        }
        unobserve() {}
    }
);

let model: DroneViewModel | null = null;
beforeEach(() => {
    rect = { ...RECT };
    observers.length = 0;
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
        () =>
            ({
                ...rect,
                x: rect.left,
                y: rect.top,
                right: rect.left + rect.width,
                bottom: rect.top + rect.height,
                toJSON() {},
            }) as DOMRect
    );
});
afterEach(() => {
    cleanup();
    model?.dispose();
    model = null;
    vi.restoreAllMocks();
});

function mount() {
    model = new DroneViewModel({ blockId: "drone-canvas-block", meta: () => ({}) } as unknown as PaneTabHostContext);
    const m = model;
    const result = render(() => <DroneView model={m} />);
    const q = <T extends HTMLElement | SVGElement = HTMLElement>(sel: string) =>
        result.container.querySelector<T>(sel)!;
    const qa = (sel: string) => Array.from(result.container.querySelectorAll<HTMLElement>(sel));
    return { m, q, qa, canvas: q(".drone-canvas"), unmount: () => result.unmount() };
}

const press = (target: EventTarget, type: string, init: MouseEventInit = {}) => {
    const e = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, ...init });
    target.dispatchEvent(e);
    return e;
};
const down = (target: EventTarget, x: number, y: number, button = 0) =>
    press(target, "pointerdown", { clientX: x, clientY: y, button });
const move = (x: number, y: number) => press(window, "pointermove", { clientX: x, clientY: y });
const up = (x = 0, y = 0) => press(window, "pointerup", { clientX: x, clientY: y });
const wheel = (target: EventTarget, deltaY: number, x = 450, y = 320) => {
    const e = new WheelEvent("wheel", { deltaY, clientX: x, clientY: y, bubbles: true, cancelable: true });
    target.dispatchEvent(e);
    return e;
};
const key = (target: EventTarget, k: string) => {
    const e = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true });
    target.dispatchEvent(e);
    return e;
};
const click = (el: HTMLElement) => el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));

const nodes = (m: DroneViewModel) => m.draftAtom().graph.nodes;
const edges = (m: DroneViewModel) => m.draftAtom().graph.edges;
const pos = (m: DroneViewModel, id: string) => nodes(m).find((n) => n.id === id)!.position;

describe("the wheel zooms the graph around the cursor", () => {
    it("zooms in by a tenth, keeping the point under the cursor where it was", () => {
        const { m, canvas } = mount();
        m.setViewport({ x: 100, y: 40, zoom: 1 });
        const e = wheel(canvas, -100, 450, 320);
        const v = m.viewportAtom();
        expect(v.zoom).toBeCloseTo(1.1);
        // the cursor is at (400, 300) in the canvas; the point under it was (300, 260)
        expect(v.x).toBeCloseTo(400 - 300 * 1.1);
        expect(v.y).toBeCloseTo(300 - 260 * 1.1);
        expect(e.defaultPrevented).toBe(true);
    });

    it("zooms out by a tenth the other way, and a zero delta zooms out too", () => {
        const { m, canvas } = mount();
        m.setViewport({ x: 100, y: 40, zoom: 1 });
        wheel(canvas, 100, 450, 320);
        expect(m.viewportAtom().zoom).toBeCloseTo(1 / 1.1);
        expect(m.viewportAtom().x).toBeCloseTo(400 - 300 / 1.1);
        expect(m.viewportAtom().y).toBeCloseTo(300 - 260 / 1.1);
        m.setViewport({ x: 0, y: 0, zoom: 1 });
        wheel(canvas, 0);
        expect(m.viewportAtom().zoom).toBeCloseTo(1 / 1.1);
    });

    it("stops at 0.2 and at 2.5", () => {
        const { m, canvas } = mount();
        m.setViewport({ x: 7, y: 9, zoom: 2.5 });
        wheel(canvas, -100);
        expect(m.viewportAtom()).toEqual({ x: 7, y: 9, zoom: 2.5 });
        m.setViewport({ x: 0, y: 0, zoom: 2.4 });
        wheel(canvas, -100);
        expect(m.viewportAtom().zoom).toBe(2.5);
        m.setViewport({ x: 7, y: 9, zoom: 0.2 });
        wheel(canvas, 100);
        expect(m.viewportAtom()).toEqual({ x: 7, y: 9, zoom: 0.2 });
        m.setViewport({ x: 0, y: 0, zoom: 0.21 });
        wheel(canvas, 100);
        expect(m.viewportAtom().zoom).toBe(0.2);
    });

    it("lets a .nowheel area inside a node scroll instead of zooming", () => {
        const { m, canvas } = mount();
        const scroller = document.createElement("div");
        scroller.className = "nowheel";
        const inner = document.createElement("span");
        scroller.appendChild(inner);
        canvas.appendChild(scroller);
        const e = wheel(inner, -100);
        expect(m.viewportAtom().zoom).toBe(1);
        expect(e.defaultPrevented).toBe(false);
    });

    it("keeps the wheel from the window's own zoom handler", () => {
        const { canvas } = mount();
        const seen = vi.fn();
        window.addEventListener("wheel", seen);
        wheel(canvas, -100);
        window.removeEventListener("wheel", seen);
        expect(seen).not.toHaveBeenCalled();
    });
});

describe("the zoom buttons and fit view", () => {
    const button = (q: ReturnType<typeof mount>["q"], title: string) => q<HTMLButtonElement>(`[title="${title}"]`);

    it("zoom in and out scale by 1.2 around the middle of the canvas", () => {
        const { m, q } = mount();
        click(button(q, "Zoom in"));
        expect(m.viewportAtom().zoom).toBeCloseTo(1.2);
        expect(m.viewportAtom().x).toBeCloseTo(400 - 400 * 1.2);
        expect(m.viewportAtom().y).toBeCloseTo(300 - 300 * 1.2);
        m.setViewport({ x: 0, y: 0, zoom: 1 });
        click(button(q, "Zoom out"));
        expect(m.viewportAtom().zoom).toBeCloseTo(1 / 1.2);
        expect(m.viewportAtom().x).toBeCloseTo(400 - 400 / 1.2);
        expect(m.viewportAtom().y).toBeCloseTo(300 - 300 / 1.2);
    });

    it("keep the point at the middle where it was when the graph is panned", () => {
        const { m, q } = mount();
        m.setViewport({ x: 100, y: -50, zoom: 2 });
        click(button(q, "Zoom in"));
        // the middle (400, 300) shows the point (150, 175); it still does at zoom 2.4
        expect(m.viewportAtom().zoom).toBeCloseTo(2.4);
        expect(m.viewportAtom().x).toBeCloseTo(400 - 150 * 2.4);
        expect(m.viewportAtom().y).toBeCloseTo(300 - 175 * 2.4);
    });

    it("stop at the zoom limits", () => {
        const { m, q } = mount();
        m.setViewport({ x: 0, y: 0, zoom: 2.4 });
        click(button(q, "Zoom in"));
        expect(m.viewportAtom().zoom).toBe(2.5);
        m.setViewport({ x: 0, y: 0, zoom: 0.21 });
        click(button(q, "Zoom out"));
        expect(m.viewportAtom().zoom).toBe(0.2);
    });

    it("fit view on an empty graph goes back to the origin at zoom 1", () => {
        const { m, q } = mount();
        m.setViewport({ x: 33, y: 44, zoom: 1.7 });
        click(button(q, "Fit view"));
        expect(m.viewportAtom()).toEqual({ x: 0, y: 0, zoom: 1 });
    });

    it("fit view frames every node with an 80px margin, centred", () => {
        const { m, q } = mount();
        m.addNode("variables", { x: 0, y: 0 });
        m.addNode("response", { x: 500, y: 300 });
        click(button(q, "Fit view"));
        // the nodes span x 0..748 (a node is 248 wide) and y 0..376 (76 tall)
        const zoom = Math.min((800 - 80) / 748, (600 - 80) / 376);
        expect(m.viewportAtom().zoom).toBeCloseTo(zoom);
        expect(m.viewportAtom().x).toBeCloseTo(400 - 374 * zoom);
        expect(m.viewportAtom().y).toBeCloseTo(300 - 188 * zoom);
    });

    it("fit view is held by the height when the graph is tall", () => {
        const { m, q } = mount();
        m.addNode("variables", { x: 0, y: 0 });
        m.addNode("response", { x: 100, y: 1000 });
        click(button(q, "Fit view"));
        const zoom = (600 - 80) / 1076;
        expect(m.viewportAtom().zoom).toBeCloseTo(zoom);
        expect(m.viewportAtom().x).toBeCloseTo(400 - 174 * zoom);
        expect(m.viewportAtom().y).toBeCloseTo(300 - 538 * zoom);
    });

    it("fit view takes nodes left of and above the origin into account", () => {
        const { m, q } = mount();
        m.addNode("variables", { x: -200, y: -100 });
        m.addNode("response", { x: 100, y: 50 });
        click(button(q, "Fit view"));
        // x -200..348, y -100..126
        const zoom = Math.min(720 / 548, 520 / 226);
        expect(m.viewportAtom().zoom).toBeCloseTo(zoom);
        expect(m.viewportAtom().x).toBeCloseTo(400 - 74 * zoom);
        expect(m.viewportAtom().y).toBeCloseTo(300 - 13 * zoom);
    });

    it("fit view frames nodes that are all right of and below the origin", () => {
        const { m, q } = mount();
        m.addNode("variables", { x: 100, y: 100 });
        click(button(q, "Fit view"));
        // one node, x 100..348 and y 100..176: its middle goes to the middle of the canvas
        expect(m.viewportAtom().zoom).toBe(2.5);
        expect(m.viewportAtom().x).toBeCloseTo(400 - 224 * 2.5);
        expect(m.viewportAtom().y).toBeCloseTo(300 - 138 * 2.5);
    });

    it("fit view frames nodes that are all left of and above the origin", () => {
        const { m, q } = mount();
        m.addNode("variables", { x: -1000, y: -1000 });
        click(button(q, "Fit view"));
        expect(m.viewportAtom().zoom).toBe(2.5);
        expect(m.viewportAtom().x).toBeCloseTo(400 - (-1000 + 124) * 2.5);
        expect(m.viewportAtom().y).toBeCloseTo(300 - (-1000 + 38) * 2.5);
    });

    it("fit view does not zoom a lone node past 2.5, nor a huge graph below 0.2", () => {
        const { m, q } = mount();
        const a = m.addNode("variables", { x: 0, y: 0 });
        click(button(q, "Fit view"));
        expect(m.viewportAtom().zoom).toBe(2.5);
        expect(m.viewportAtom().x).toBeCloseTo(400 - 124 * 2.5);
        expect(m.viewportAtom().y).toBeCloseTo(300 - 38 * 2.5);
        m.moveNode(a.id, { x: 0, y: 0 });
        m.addNode("response", { x: 10000, y: 0 });
        click(button(q, "Fit view"));
        expect(m.viewportAtom().zoom).toBe(0.2);
    });
});

describe("the viewport", () => {
    it("is the transform of the viewport layer", () => {
        const { m, q } = mount();
        m.setViewport({ x: 10, y: 20, zoom: 1.5 });
        const style = q(".drone-viewport").style;
        expect(style.transform).toBe("translate(10px, 20px) scale(1.5)");
        expect(style.transformOrigin).toBe("0 0");
    });
});

describe("dragging a node", () => {
    it("moves it by the pointer's travel divided by the zoom at the start", () => {
        const { m, q } = mount();
        const n = m.addNode("api", { x: 100, y: 100 });
        m.setViewport({ zoom: 2 });
        down(q(".drone-node-header"), 200, 200);
        move(260, 230);
        expect(pos(m, n.id)).toEqual({ x: 130, y: 115 });
        // zooming mid-drag does not change the scale of the drag
        m.setViewport({ zoom: 1 });
        move(300, 300);
        expect(pos(m, n.id)).toEqual({ x: 150, y: 150 });
    });

    it("selects the node, clears a selected edge, and focuses the canvas", () => {
        const { m, q, canvas } = mount();
        const a = m.addNode("variables", { x: 0, y: 0 });
        const b = m.addNode("response", { x: 400, y: 0 });
        m.connect({ source: a.id, target: b.id });
        m.setSelectedEdge(edges(m)[0].id);
        down(q(`.drone-node:nth-of-type(2) .drone-node-header`), 5, 5);
        expect(m.selectedAtom()).toBe(nodes(m)[1].id);
        expect(m.selectedEdgeAtom()).toBeNull();
        expect(document.activeElement).toBe(canvas);
        up();
    });

    it("stops following the pointer on release", () => {
        const { m, q } = mount();
        const n = m.addNode("api", { x: 100, y: 100 });
        down(q(".drone-node-header"), 200, 200);
        move(210, 210);
        up();
        move(500, 500);
        expect(pos(m, n.id)).toEqual({ x: 110, y: 110 });
    });

    it("needs the left button", () => {
        const { m, q } = mount();
        const n = m.addNode("api", { x: 100, y: 100 });
        down(q(".drone-node-header"), 200, 200, 2);
        move(300, 300);
        expect(pos(m, n.id)).toEqual({ x: 100, y: 100 });
        expect(m.selectedAtom()).toBeNull();
    });

    it("does not start from a port or from the remove button", () => {
        const { m, q } = mount();
        const n = m.addNode("api", { x: 100, y: 100 });
        m.setViewport({ x: 5, y: 6 });
        down(q(".drone-port--in"), 200, 200);
        move(300, 300);
        up();
        down(q(".drone-node-close"), 200, 200);
        move(300, 300);
        up();
        expect(pos(m, n.id)).toEqual({ x: 100, y: 100 });
        expect(m.selectedAtom()).toBeNull();
        // and neither press fell through to a pan of the canvas
        expect(m.viewportAtom()).toEqual({ x: 5, y: 6, zoom: 1 });
    });

    it("does not pan the canvas as well", () => {
        const { m, q } = mount();
        m.addNode("api", { x: 100, y: 100 });
        down(q(".drone-node-header"), 200, 200);
        move(260, 240);
        up();
        expect(m.viewportAtom()).toEqual({ x: 0, y: 0, zoom: 1 });
    });

    it("is dropped when the pane is closed mid-drag", () => {
        const { m, q, unmount } = mount();
        const n = m.addNode("api", { x: 100, y: 100 });
        down(q(".drone-node-header"), 200, 200);
        unmount();
        move(300, 300);
        expect(pos(m, n.id)).toEqual({ x: 100, y: 100 });
    });
});

describe("panning", () => {
    it("drags the viewport by the pointer's travel", () => {
        const { m, canvas } = mount();
        m.setViewport({ x: 10, y: 20, zoom: 1.5 });
        down(canvas, 100, 100);
        move(130, 150);
        expect(m.viewportAtom()).toEqual({ x: 40, y: 70, zoom: 1.5 });
        move(90, 80);
        expect(m.viewportAtom()).toEqual({ x: 0, y: 0, zoom: 1.5 });
        up();
        move(500, 500);
        expect(m.viewportAtom()).toEqual({ x: 0, y: 0, zoom: 1.5 });
    });

    it("starts from the viewport layer, the edges layer and the canvas itself, not from other things", () => {
        const { m, q, canvas } = mount();
        for (const el of [canvas, q(".drone-viewport"), q(".drone-canvas-edges")]) {
            m.setViewport({ x: 0, y: 0 });
            down(el, 0, 0);
            move(10, 10);
            up();
            expect(m.viewportAtom().x).toBe(10);
        }
        m.setViewport({ x: 0, y: 0 });
        down(q(".drone-canvas-hint"), 0, 0);
        move(10, 10);
        up();
        expect(m.viewportAtom().x).toBe(0);
        down(q(".drone-ctrl"), 0, 0);
        move(10, 10);
        up();
        expect(m.viewportAtom().x).toBe(0);
    });

    it("starts from a plain svg too", () => {
        const { m, canvas } = mount();
        const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
        canvas.appendChild(svg);
        down(svg, 0, 0);
        move(10, 0);
        up();
        expect(m.viewportAtom().x).toBe(10);
    });

    it("takes the middle button anywhere, and not the right one", () => {
        const { m, q, canvas } = mount();
        m.addNode("api", { x: 100, y: 100 });
        down(q(".drone-node-header"), 0, 0, 1);
        move(10, 5);
        up();
        expect(m.viewportAtom()).toEqual({ x: 10, y: 5, zoom: 1 });
        m.setViewport({ x: 0, y: 0 });
        down(canvas, 0, 0, 2);
        move(10, 10);
        up();
        expect(m.viewportAtom().x).toBe(0);
    });

    it("treats a press that stays within 2px as a click and deselects", () => {
        const { m, canvas } = mount();
        const n = m.addNode("api", { x: 100, y: 100 });
        for (const [dx, dy] of [
            [0, 0],
            [2, 0],
            [0, 2],
            [-2, -2],
        ]) {
            m.setSelected(n.id);
            down(canvas, 100, 100);
            move(100 + dx, 100 + dy);
            up();
            expect(m.selectedAtom()).toBeNull();
        }
    });

    it("keeps the selection after a real drag in any direction", () => {
        const { m, canvas } = mount();
        const n = m.addNode("api", { x: 100, y: 100 });
        for (const [dx, dy] of [
            [3, 0],
            [0, 3],
            [-3, 0],
            [0, -3],
        ]) {
            m.setSelected(n.id);
            down(canvas, 100, 100);
            move(100 + dx, 100 + dy);
            up();
            expect(m.selectedAtom()).toBe(n.id);
        }
    });

    it("remembers a drag that came back to where it started", () => {
        const { m, canvas } = mount();
        const n = m.addNode("api", { x: 100, y: 100 });
        m.setSelected(n.id);
        down(canvas, 100, 100);
        move(150, 100);
        move(100, 100);
        up();
        expect(m.selectedAtom()).toBe(n.id);
    });

    it("is dropped when the pane is closed mid-pan", () => {
        const { m, canvas, unmount } = mount();
        down(canvas, 0, 0);
        unmount();
        move(50, 50);
        expect(m.viewportAtom()).toEqual({ x: 0, y: 0, zoom: 1 });
    });
});

describe("dropping a chip on the canvas", () => {
    function drop(canvas: HTMLElement, x: number, y: number, mime?: string) {
        const e = new Event("drop", { bubbles: true, cancelable: true });
        Object.defineProperty(e, "dataTransfer", {
            value: {
                types: mime ? ["application/x-drone-kind"] : [],
                getData: (k: string) => (k === "application/x-drone-kind" && mime ? mime : ""),
            },
        });
        Object.assign(e, { clientX: x, clientY: y });
        canvas.dispatchEvent(e);
        return e;
    }

    it("puts the node's body under the cursor, in canvas coordinates at the current zoom", () => {
        const { m, canvas } = mount();
        m.setViewport({ x: 100, y: 50, zoom: 2 });
        drop(canvas, 450, 370, "agent");
        // (450 - 50 - 100) / 2 = 150 and (370 - 20 - 50) / 2 = 150, then half a node left and 20 up
        expect(nodes(m)).toHaveLength(1);
        expect(nodes(m)[0].data.kind).toBe("agent");
        expect(nodes(m)[0].position).toEqual({ x: 150 - 124, y: 150 - 20 });
    });

    it("drops nothing, but still takes the drop, when the kind is empty", () => {
        const { m, canvas } = mount();
        const e = new Event("drop", { bubbles: true, cancelable: true });
        Object.defineProperty(e, "dataTransfer", {
            value: { types: ["application/x-drone-kind"], getData: () => "" },
        });
        canvas.dispatchEvent(e);
        expect(nodes(m)).toHaveLength(0);
        expect(e.defaultPrevented).toBe(true);
    });

    it("takes the drop and ends the chip's drag", () => {
        const { m, q, canvas } = mount();
        const chip = q(".drone-chip");
        const start = new Event("dragstart", { bubbles: true, cancelable: true });
        Object.defineProperty(start, "dataTransfer", { value: { setData() {}, effectAllowed: "" } });
        chip.dispatchEvent(start);
        expect(session()?.kind).toBe("drone-kind");
        const e = drop(canvas, 100, 100);
        expect(e.defaultPrevented).toBe(true);
        expect(nodes(m)).toHaveLength(1);
        expect(session()).toBeNull();
    });

    it("leaves a drop of something else alone", () => {
        const { m, canvas } = mount();
        const e = new Event("drop", { bubbles: true, cancelable: true });
        Object.defineProperty(e, "dataTransfer", { value: { types: ["Files"], getData: () => "agent" } });
        canvas.dispatchEvent(e);
        expect(e.defaultPrevented).toBe(false);
        expect(nodes(m)).toHaveLength(0);
    });
});

describe("the canvas's size", () => {
    it("is read at mount and on every resize, so a chip click lands in the middle", () => {
        const { m, q } = mount();
        click(q(".drone-chip"));
        // the middle of 800 x 600, less half a node and 30
        expect(nodes(m)[0].position).toEqual({ x: 400 - 124, y: 300 - 30 });
        rect = { ...rect, width: 400, height: 200 };
        observers[0].cb();
        click(q(".drone-chip"));
        expect(nodes(m)[1].position).toEqual({ x: 200 - 124, y: 100 - 30 });
    });

    it("is the canvas element that is watched, until the pane is closed", () => {
        const { canvas, unmount } = mount();
        expect(observers).toHaveLength(1);
        expect(observers[0].observe).toHaveBeenCalledWith(canvas);
        unmount();
        expect(observers[0].disconnect).toHaveBeenCalledTimes(1);
    });
});

describe("focus", () => {
    it("moves to the canvas even when a handler below it stops the press natively", () => {
        const { q, canvas } = mount();
        const blocker = document.createElement("button");
        blocker.addEventListener("pointerdown", (e) => e.stopPropagation());
        canvas.appendChild(blocker);
        (document.activeElement as HTMLElement | null)?.blur();
        down(blocker, 5, 5);
        expect(document.activeElement).toBe(canvas);
        expect(q(".drone-canvas")).toBe(canvas);
    });

    it("is no longer taken once the pane is closed", () => {
        const { canvas, unmount } = mount();
        const remove = vi.spyOn(canvas, "removeEventListener");
        unmount();
        expect(remove).toHaveBeenCalledWith("pointerdown", expect.any(Function), true);
    });

    it("moves to the canvas on any press inside it, even one a node stops", () => {
        const { m, q, canvas } = mount();
        m.addNode("api", { x: 0, y: 0 });
        (document.activeElement as HTMLElement | null)?.blur();
        down(q(".drone-node-header"), 5, 5);
        up();
        expect(document.activeElement).toBe(canvas);
    });
});

describe("wiring one port to another", () => {
    function wired() {
        const t = mount();
        const a = t.m.addNode("variables", { x: 0, y: 0 });
        const b = t.m.addNode("response", { x: 400, y: 0 });
        const outPort = t.q(`.drone-port--out[data-node="${a.id}"]`);
        const inPort = t.q(`.drone-port--in[data-node="${b.id}"]`);
        const hit = (el: Element | null) => (document.elementFromPoint = vi.fn(() => el));
        return { ...t, a, b, outPort, inPort, hit };
    }
    const preview = (q: ReturnType<typeof mount>["q"]) => q(".drone-edge-preview")?.getAttribute("d") ?? null;

    it("starts a preview at the output port and follows the pointer in canvas coordinates", () => {
        const { q, outPort } = wired();
        expect(preview(q)).toBeNull();
        down(outPort, 0, 0);
        expect(preview(q)).toBe("M 248 26 C 288 26, 208 26, 248 26");
        move(850, 120);
        // the cursor is at (800, 100) in the canvas
        expect(preview(q)).toBe("M 248 26 C 524 26, 524 100, 800 100");
        move(350, 120);
        // a short run keeps the 40px minimum curve
        expect(preview(q)).toBe("M 248 26 C 288 26, 260 100, 300 100");
    });

    it("joins the port under the pointer on release, and the preview goes", () => {
        const { m, q, a, b, outPort, inPort, hit } = wired();
        down(outPort, 0, 0);
        hit(inPort);
        up(300, 200);
        expect(document.elementFromPoint).toHaveBeenCalledWith(300, 200);
        expect(edges(m)).toHaveLength(1);
        expect(edges(m)[0]).toMatchObject({ source: a.id, sourceHandle: "out", target: b.id, targetHandle: "in" });
        expect(preview(q)).toBeNull();
    });

    it("uses the handle of the port it started on", () => {
        const { m, q, b, inPort, hit } = wired();
        const c = m.addNode("condition", { x: 0, y: 200 });
        const falsePort = q(`.drone-port--out[data-node="${c.id}"][data-handle="false"]`);
        down(falsePort, 0, 0);
        // the second of two outputs sits 10px below the first
        expect(preview(q)).toBe("M 248 236 C 288 236, 208 236, 248 236");
        hit(inPort);
        up();
        expect(edges(m)[0]).toMatchObject({ source: c.id, sourceHandle: "false", target: b.id });
    });

    it("takes the target's own handle, and 'in' when the port names none", () => {
        const { m, a, b, outPort, hit } = wired();
        const bare = document.createElement("div");
        bare.className = "drone-port--in";
        bare.dataset.node = b.id;
        document.body.appendChild(bare);
        down(outPort, 0, 0);
        hit(bare);
        up();
        bare.remove();
        expect(edges(m)[0]).toMatchObject({ source: a.id, target: b.id, targetHandle: "in" });
        expect(edges(m)).toHaveLength(1);
    });

    it("takes the handle an input port names, whatever it is", () => {
        const { m, a, b, outPort, hit } = wired();
        const alt = document.createElement("div");
        alt.className = "drone-port--in";
        alt.dataset.node = b.id;
        alt.dataset.handle = "alt";
        document.body.appendChild(alt);
        down(outPort, 0, 0);
        hit(alt);
        up();
        alt.remove();
        expect(edges(m)[0]).toMatchObject({ source: a.id, target: b.id, targetHandle: "alt" });
    });

    it("does not even try a port that names no node", () => {
        const { m, outPort, hit } = wired();
        const connect = vi.spyOn(m, "connect");
        const anon = document.createElement("div");
        anon.className = "drone-port--in";
        document.body.appendChild(anon);
        down(outPort, 0, 0);
        hit(anon);
        up();
        anon.remove();
        expect(connect).not.toHaveBeenCalled();
    });

    it("finds an input port from something inside it", () => {
        const { m, outPort, inPort, hit } = wired();
        const inner = document.createElement("i");
        inPort.appendChild(inner);
        down(outPort, 0, 0);
        hit(inner);
        up();
        expect(edges(m)).toHaveLength(1);
    });

    it("joins nothing when released over empty space, a node, or an output port", () => {
        const { m, q, canvas, outPort, hit } = wired();
        down(outPort, 0, 0);
        hit(canvas);
        up();
        expect(edges(m)).toHaveLength(0);
        expect(preview(q)).toBeNull();
        down(outPort, 0, 0);
        hit(null);
        up();
        expect(edges(m)).toHaveLength(0);
        down(outPort, 0, 0);
        hit(outPort);
        up();
        expect(edges(m)).toHaveLength(0);
        expect(preview(q)).toBeNull();
    });

    it("joins nothing the model refuses, and still clears the preview", () => {
        const { m, q, hit } = wired();
        const c = m.addNode("agent", { x: 0, y: 200 });
        down(q(`.drone-port--out[data-node="${c.id}"]`), 0, 0);
        hit(q(`.drone-port--in[data-node="${c.id}"]`));
        up();
        expect(edges(m)).toHaveLength(0);
        expect(preview(q)).toBeNull();
    });

    it("does not select or drag the node it starts from", () => {
        const { m, a, outPort } = wired();
        down(outPort, 0, 0);
        move(100, 100);
        up();
        expect(m.selectedAtom()).toBeNull();
        expect(pos(m, a.id)).toEqual({ x: 0, y: 0 });
        expect(m.viewportAtom()).toEqual({ x: 0, y: 0, zoom: 1 });
    });

    it("needs the left button", () => {
        const { q, outPort } = wired();
        down(outPort, 0, 0, 2);
        expect(preview(q)).toBeNull();
    });

    it("is cancelled by Escape, which also clears the selection", () => {
        const { m, q, a, canvas, outPort, inPort, hit } = wired();
        m.setSelected(a.id);
        down(outPort, 0, 0);
        const e = key(canvas, "Escape");
        expect(preview(q)).toBeNull();
        expect(m.selectedAtom()).toBeNull();
        hit(inPort);
        up();
        expect(edges(m)).toHaveLength(0);
        expect(e.defaultPrevented).toBe(false);
    });

    it("is dropped when the pane is closed mid-wire", () => {
        const { m, outPort, inPort, hit, unmount } = wired();
        down(outPort, 0, 0);
        unmount();
        hit(inPort);
        up();
        expect(edges(m)).toHaveLength(0);
    });
});

describe("the edges", () => {
    function two() {
        const t = mount();
        const a = t.m.addNode("variables", { x: 0, y: 0 });
        const b = t.m.addNode("agent", { x: 500, y: 100 });
        return { ...t, a, b };
    }

    it("is drawn from the output port to the input port", () => {
        const { m, q, qa, a, b } = two();
        m.connect({ source: a.id, target: b.id });
        // out at (248, 26); in at (500, 126): a 252px run, so half of it is the curve
        expect(qa(".drone-edge").map((p) => p.getAttribute("d"))).toEqual(["M 248 26 C 374 26, 374 126, 500 126"]);
        expect(qa(".drone-edge-hit").map((p) => p.getAttribute("d"))).toEqual(["M 248 26 C 374 26, 374 126, 500 126"]);
        expect(q(".drone-edge--selected")).toBeNull();
    });

    it("leaves out an edge to a node that is not there, and draws the rest", () => {
        const { m, qa, a, b } = two();
        m.addEdge({ source: a.id, target: "ghost" });
        m.addEdge({ source: "ghost", target: b.id });
        expect(qa(".drone-edge")).toHaveLength(0);
        const c = m.addNode("response", { x: 500, y: 400 });
        m.connect({ source: a.id, target: c.id });
        expect(qa(".drone-edge")).toHaveLength(1);
    });

    it("curves by half the run even when it runs right to left", () => {
        const { m, qa, a, b } = two();
        m.moveNode(a.id, { x: 600, y: 0 });
        m.moveNode(b.id, { x: 0, y: 100 });
        m.connect({ source: a.id, target: b.id });
        // out at (848, 26), in at (0, 126): an 848px run backwards
        expect(qa(".drone-edge")[0].getAttribute("d")).toBe("M 848 26 C 1272 26, -424 126, 0 126");
    });

    it("moves with its nodes", () => {
        const { m, qa, a, b } = two();
        m.connect({ source: a.id, target: b.id });
        m.moveNode(a.id, { x: 10, y: 20 });
        expect(qa(".drone-edge")[0].getAttribute("d")).toBe("M 258 46 C 379 46, 379 126, 500 126");
    });

    it("uses the first port when the edge names none, or one the node does not have", () => {
        const { m, qa, a, b } = two();
        const c = m.addNode("condition", { x: 0, y: 300 });
        m.addEdge({ source: c.id, target: b.id });
        m.addEdge({ source: c.id, sourceHandle: "nope", target: b.id, targetHandle: "nope" });
        m.addEdge({ source: a.id, sourceHandle: undefined, target: b.id, targetHandle: undefined });
        // the condition's first of two outputs: 26 - 10 = 16 below its top
        const ds = qa(".drone-edge").map((p) => p.getAttribute("d"));
        expect(ds[0]).toBe("M 248 316 C 374 316, 374 126, 500 126");
        expect(ds[1]).toBe(ds[0]);
    });

    it("is selected by a press on its wide hit path, which clears the node selection", () => {
        const { m, q, qa, a, b, canvas } = two();
        m.connect({ source: a.id, target: b.id });
        m.setSelected(a.id);
        const e = down(q(".drone-edge-hit"), 5, 5);
        expect(m.selectedEdgeAtom()).toBe(edges(m)[0].id);
        expect(m.selectedAtom()).toBeNull();
        expect(qa(".drone-edge--selected")).toHaveLength(1);
        expect(e.defaultPrevented).toBe(false);
        // the press did not also start a pan
        move(50, 50);
        up();
        expect(m.viewportAtom()).toEqual({ x: 0, y: 0, zoom: 1 });
        expect(document.activeElement).toBe(canvas);
    });

    it("marks only the selected edge", () => {
        const { m, qa, a, b } = two();
        const c = m.addNode("response", { x: 500, y: 400 });
        m.connect({ source: a.id, target: b.id });
        m.connect({ source: b.id, target: c.id });
        m.setSelectedEdge(edges(m)[1].id);
        const flags = qa(".drone-edge").map((p) => p.classList.contains("drone-edge--selected"));
        expect(flags).toEqual([false, true]);
    });

    it("goes when a node it joins is removed", () => {
        const { m, qa, a, b } = two();
        m.connect({ source: a.id, target: b.id });
        m.removeNode(b.id);
        expect(qa(".drone-edge")).toHaveLength(0);
    });
});

describe("the nodes", () => {
    it("sit at their position and mark the selected one", () => {
        const { m, qa } = mount();
        const a = m.addNode("api", { x: 12, y: 34 });
        m.addNode("response", { x: 56, y: 78 });
        const [first, second] = qa(".drone-node");
        expect(first.style.left).toBe("12px");
        expect(first.style.top).toBe("34px");
        expect(second.style.left).toBe("56px");
        expect(second.style.top).toBe("78px");
        expect(qa(".drone-node--selected")).toHaveLength(0);
        m.setSelected(a.id);
        expect(qa(".drone-node--selected")).toEqual([first]);
    });

    it("show a summary until selected, then their fields", () => {
        const { m, q } = mount();
        const a = m.addNode("api", { x: 0, y: 0 });
        expect(q(".drone-node-summary")).not.toBeNull();
        expect(q(".drone-node-fields")).toBeNull();
        m.setSelected(a.id);
        expect(q(".drone-node-summary")).toBeNull();
        expect(q(".drone-node-fields")).not.toBeNull();
    });

    it("have a port per input and per output, in the right places", () => {
        const { m, q, qa } = mount();
        m.addNode("variables", { x: 0, y: 0 });
        expect(qa(".drone-port--in")).toHaveLength(0);
        expect(qa(".drone-port--out")).toHaveLength(1);
        m.addNode("condition", { x: 0, y: 200 });
        expect(qa(".drone-port--in")).toHaveLength(1);
        expect(qa(".drone-port--out")).toHaveLength(3);
        m.addNode("response", { x: 0, y: 400 });
        expect(qa(".drone-port--out")).toHaveLength(3);
        expect(qa(".drone-port--in").map((p) => p.style.top)).toEqual(["20px", "20px"]);
        const cond = nodes(m)[1].id;
        const tops = qa(`.drone-port--out[data-node="${cond}"]`).map((p) => p.style.top);
        expect(tops).toEqual(["10px", "30px"]);
        expect(q(`.drone-port--out[data-node="${cond}"]`).dataset.handle).toBe("true");
    });

    it("colour a condition's true and false outputs, and the rest like the node", () => {
        const { m, qa } = mount();
        const c = m.addNode("condition", { x: 0, y: 0 });
        m.addNode("agent", { x: 0, y: 200 });
        const bg = (p: HTMLElement) => p.style.background;
        const [t, f, o] = qa(".drone-port--out");
        expect(t.dataset.node).toBe(c.id);
        expect(bg(t)).toContain("var(--success-color)");
        expect(bg(f)).toContain("var(--secondary-text-color)");
        expect(bg(o)).toBe("rgb(59, 130, 246)");
    });

    it("are removed by their close button without being selected first", () => {
        const { m, q } = mount();
        m.addNode("api", { x: 0, y: 0 });
        m.addNode("response", { x: 400, y: 0 });
        click(q(".drone-node-close"));
        expect(nodes(m)).toHaveLength(1);
        expect(nodes(m)[0].data.kind).toBe("response");
        expect(m.selectedAtom()).toBeNull();
    });

    it("explain how to start while the graph is empty", () => {
        const { m, q } = mount();
        expect(q(".drone-canvas-hint").textContent).toContain("Drag a block from the top bar");
        m.addNode("api", { x: 0, y: 0 });
        expect(q(".drone-canvas-hint")).toBeNull();
    });
});

describe("the keys", () => {
    it("Delete removes the selected node, and Backspace does too", () => {
        const { m, canvas } = mount();
        const a = m.addNode("api", { x: 0, y: 0 });
        const b = m.addNode("response", { x: 400, y: 0 });
        m.setSelected(a.id);
        const e = key(canvas, "Delete");
        expect(e.defaultPrevented).toBe(true);
        expect(nodes(m).map((n) => n.id)).toEqual([b.id]);
        m.setSelected(b.id);
        key(canvas, "Backspace");
        expect(nodes(m)).toHaveLength(0);
    });

    it("removes the selected edge before the selected node", () => {
        const { m, canvas } = mount();
        const a = m.addNode("variables", { x: 0, y: 0 });
        const b = m.addNode("response", { x: 400, y: 0 });
        m.connect({ source: a.id, target: b.id });
        m.setSelectedEdge(edges(m)[0].id);
        m.setSelected(a.id);
        const first = key(canvas, "Delete");
        expect(first.defaultPrevented).toBe(true);
        expect(edges(m)).toHaveLength(0);
        expect(nodes(m)).toHaveLength(2);
        key(canvas, "Delete");
        expect(nodes(m).map((n) => n.id)).toEqual([b.id]);
    });

    it("leaves the key alone when nothing is selected, and ignores other keys", () => {
        const { m, canvas } = mount();
        const a = m.addNode("api", { x: 0, y: 0 });
        const e = key(canvas, "Delete");
        expect(e.defaultPrevented).toBe(false);
        m.setSelected(a.id);
        const other = key(canvas, "a");
        expect(other.defaultPrevented).toBe(false);
        expect(nodes(m)).toHaveLength(1);
        expect(m.selectedAtom()).toBe(a.id);
    });

    it("Escape clears the node and edge selection", () => {
        const { m, canvas } = mount();
        const a = m.addNode("variables", { x: 0, y: 0 });
        const b = m.addNode("response", { x: 400, y: 0 });
        m.connect({ source: a.id, target: b.id });
        m.setSelectedEdge(edges(m)[0].id);
        m.setSelected(a.id);
        key(canvas, "Escape");
        expect(m.selectedAtom()).toBeNull();
        expect(m.selectedEdgeAtom()).toBeNull();
    });

    it.each(["input", "textarea", "select"])("are not taken from a %s inside the canvas", (tag) => {
        const { m, canvas } = mount();
        const a = m.addNode("api", { x: 0, y: 0 });
        m.setSelected(a.id);
        const field = document.createElement(tag);
        canvas.appendChild(field);
        const del = key(field, "Backspace");
        const esc = key(field, "Escape");
        expect(del.defaultPrevented).toBe(false);
        expect(esc.defaultPrevented).toBe(false);
        expect(nodes(m)).toHaveLength(1);
        expect(m.selectedAtom()).toBe(a.id);
    });
});
