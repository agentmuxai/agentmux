// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentTopBand } from "./AgentTopBand";
import { registerTopBandItem, topBandItems, type TopBandItem } from "./top-band-registry";

// jsdom has no layout: a probe reports the width its content declares (data-w), and the row the width the test sets.
let rowWidth = 1000;
const observers: { cb: ResizeObserverCallback; seen: Set<Element> }[] = [];
const realRect = Element.prototype.getBoundingClientRect;

beforeEach(() => {
    rowWidth = 1000;
    observers.length = 0;
    Element.prototype.getBoundingClientRect = function (this: Element) {
        const width = this.classList.contains("agent-top-band-probe")
            ? Number(this.firstElementChild?.getAttribute("data-w") ?? 0)
            : this.classList.contains("agent-top-band-row")
              ? rowWidth
              : 0;
        return { width, height: 0, top: 0, left: 0, right: width, bottom: 0, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    };
    vi.stubGlobal(
        "ResizeObserver",
        class {
            seen = new Set<Element>();
            constructor(public cb: ResizeObserverCallback) {
                observers.push(this);
            }
            observe(el: Element) {
                this.seen.add(el);
            }
            unobserve(el: Element) {
                this.seen.delete(el);
            }
            disconnect() {
                this.seen.clear();
            }
        }
    );
});

afterEach(() => {
    cleanup();
    Element.prototype.getBoundingClientRect = realRect;
    vi.unstubAllGlobals();
});

/** Tell the band the row changed size, as the browser would. */
const resizeRow = (view: { container: HTMLElement }, width: number) => {
    rowWidth = width;
    const row = view.container.querySelector(".agent-top-band-row")!;
    for (const o of observers) if (o.seen.has(row)) o.cb([{ target: row } as ResizeObserverEntry], o as never);
};

const chip = (id: string, over: Partial<TopBandItem> = {}): TopBandItem => ({
    id,
    slot: "stats",
    priority: 10,
    full: () => <span data-w="80" data-testid={`${id}-full`} />,
    compact: () => <span data-w="30" data-testid={`${id}-compact`} />,
    ...over,
});
const find = (): TopBandItem => ({
    id: "find",
    slot: "controls",
    priority: 100,
    full: (ctx) => <span data-w="300" data-testid={ctx.measuring ? "find-probe" : "find"} />,
});

let n = 0;
const mount = (...items: TopBandItem[]) => {
    const blockId = `b${++n}`;
    const disposers = items.map((i) => registerTopBandItem(blockId, i));
    const view = render(() => <AgentTopBand blockId={blockId} zoom={() => 1} />);
    return { view, blockId, disposers };
};
const live = (view: { container: HTMLElement }, id: string) =>
    view.container.querySelector(`.agent-top-band-row [data-band-item="${id}"]`);

describe("AgentTopBand", () => {
    it("draws nothing while nothing is registered, and appears with the first item", () => {
        const { view, blockId } = mount();
        expect(view.container.querySelector(".agent-top-band")).toBeNull();
        registerTopBandItem(blockId, find());
        expect(view.container.querySelector(".agent-top-band")).toBeTruthy();
        expect(live(view, "find")).toBeTruthy();
    });

    it("puts each item in its slot, in the fixed order, in a box of its own", () => {
        const { view } = mount(find(), chip("readout"), chip("who", { slot: "identity" }));
        const slots = Array.from(view.container.querySelectorAll(".agent-top-band-row > .agent-top-band-slot")).map(
            (s) => s.getAttribute("data-slot")
        );
        expect(slots).toEqual(["identity", "stats", "controls"]);
        expect(view.container.querySelectorAll(".agent-top-band-row .agent-top-band-item")).toHaveLength(3);
    });

    it("draws a hidden measuring copy of every form, inert and out of the accessibility tree", () => {
        const { view } = mount(find(), chip("readout"));
        const probes = view.container.querySelector(".agent-top-band-probes")!;
        expect(probes.getAttribute("aria-hidden")).toBe("true");
        expect(probes.querySelectorAll(".agent-top-band-probe")).toHaveLength(3);
        for (const p of Array.from(probes.querySelectorAll(".agent-top-band-probe"))) expect(p.hasAttribute("inert")).toBe(true);
        expect(view.container.querySelector("[data-testid=find-probe]")).toBeTruthy();
        expect(view.container.querySelectorAll("[data-testid=find]")).toHaveLength(1);
    });

    it("keeps every item full while the row is wide enough, and compacts the readout when it is not", () => {
        const { view } = mount(find(), chip("readout"));
        expect(live(view, "readout")?.getAttribute("data-form")).toBe("full");
        // 300 + 8 + 80 = 388 needed
        resizeRow(view, 380);
        expect(live(view, "readout")?.getAttribute("data-form")).toBe("compact");
        expect(view.container.querySelector(".agent-top-band-row [data-testid=readout-compact]")).toBeTruthy();
        expect(live(view, "find")?.getAttribute("data-form")).toBe("full");
        resizeRow(view, 400);
        expect(live(view, "readout")?.getAttribute("data-form")).toBe("full");
    });

    it("drops an item with no compact form when compacting is not enough, and never the controls", () => {
        const { view } = mount(find(), chip("note", { compact: undefined, full: () => <span data-w="80" /> }));
        resizeRow(view, 200);
        expect(live(view, "note")).toBeNull();
        expect(live(view, "find")).toBeTruthy();
        resizeRow(view, 400);
        expect(live(view, "note")).toBeTruthy();
    });

    it("does not remount a control when the layout changes around it", () => {
        const { view } = mount(find(), chip("readout"));
        const before = live(view, "find")?.firstElementChild;
        resizeRow(view, 380);
        resizeRow(view, 500);
        expect(live(view, "find")?.firstElementChild).toBe(before);
    });

    it("takes an item out when it is disposed, and replaces one registered again under the same id", () => {
        const { view, blockId, disposers } = mount(find());
        const again = registerTopBandItem(blockId, { ...find(), priority: 50 });
        expect(topBandItems(blockId)()).toHaveLength(1);
        disposers[0]();
        expect(topBandItems(blockId)()).toHaveLength(1);
        again();
        expect(topBandItems(blockId)()).toHaveLength(0);
        expect(view.container.querySelector(".agent-top-band")).toBeNull();
    });
});
