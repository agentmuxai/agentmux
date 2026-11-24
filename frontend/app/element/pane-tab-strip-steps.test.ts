// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createStripSteps, stripRoom } from "./pane-tab-strip-steps";

/** A header range (`rangeWidth` wide) holding: a 45px item, the tab strip holder, a 0px spacer, a 45px item. */
function header(rangeWidth: number, addWidth: number, labels: Array<{ scroll: number; client: number }>) {
    const range = document.createElement("div");
    range.className = "block-frame-header-scroll";
    const host = document.createElement("div");
    host.className = "block-frame-header-content";
    host.style.columnGap = "8px";
    const pre = document.createElement("div");
    const holder = document.createElement("div");
    holder.className = "block-frame-default-header-tabstrip";
    const strip = document.createElement("div");
    strip.className = "pane-tab-strip";
    const add = document.createElement("div");
    add.className = "pane-tab-strip-add-tip";
    strip.append(add);
    labels.forEach((l) => {
        const label = document.createElement("span");
        label.className = "pane-tab-label";
        Object.defineProperty(label, "scrollWidth", { value: l.scroll });
        Object.defineProperty(label, "clientWidth", { value: l.client });
        strip.prepend(label);
    });
    holder.append(strip);
    const spacer = document.createElement("div");
    const post = document.createElement("div");
    host.append(pre, holder, spacer, post);
    range.append(host);
    document.body.append(range);
    Object.defineProperty(range, "clientWidth", { value: rangeWidth });
    const widths = new Map<Element, number>([[pre, 45], [spacer, 0], [post, 45], [add, addWidth]]);
    for (const [el, w] of widths) {
        el.getBoundingClientRect = () => ({ width: w }) as DOMRect;
        Object.defineProperty(el, "offsetParent", { value: document.body });
    }
    Object.defineProperty(holder, "offsetParent", { value: document.body });
    return { range, host, holder, strip, pre };
}

// jsdom has no layout and ignores `column-gap`: say what the header's computed style would be.
const stubStyle = () =>
    vi.spyOn(window, "getComputedStyle").mockImplementation(
        (el) => ({ columnGap: "8px", marginLeft: "0px", marginRight: "0px", flexGrow: (el as HTMLElement).dataset?.grow ?? "0" }) as CSSStyleDeclaration
    );

describe("stripRoom", () => {
    beforeEach(() => {
        stubStyle();
    });
    afterEach(() => {
        vi.restoreAllMocks();
        document.body.replaceChildren();
    });

    it("is the range's width less the other items in it and the gaps between them", () => {
        const { strip } = header(600, 28, []);
        // three other items (45, 0, 45) and three gaps of 8
        expect(stripRoom(strip)).toBe(600 - 90 - 24);
    });

    it("counts a growing item by its contents, not by the width it has grown to", () => {
        const { strip, pre } = header(600, 28, []);
        pre.dataset.grow = "1";
        pre.getBoundingClientRect = () => ({ width: 300 }) as DOMRect; // grown into room the strip left
        const inner = document.createElement("div");
        inner.getBoundingClientRect = () => ({ width: 30 }) as DOMRect;
        Object.defineProperty(inner, "offsetParent", { value: document.body });
        pre.append(inner);
        // 30 (its contents) + 0 + 45 and three gaps of 8
        expect(stripRoom(strip)).toBe(600 - 75 - 24);
    });

    it("is undefined while the header has no width", () => {
        const { strip } = header(0, 28, []);
        expect(stripRoom(strip)).toBeUndefined();
    });

    it("is the parent's width for a strip outside a header", () => {
        const parent = document.createElement("div");
        const strip = document.createElement("div");
        parent.append(strip);
        document.body.append(parent);
        Object.defineProperty(parent, "clientWidth", { value: 321 });
        expect(stripRoom(strip)).toBe(321);
    });
});

describe("createStripSteps", () => {
    beforeEach(() => {
        stubStyle();
        vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "setTimeout", "clearTimeout"] });
    });
    afterEach(() => {
        vi.useRealTimers();
        vi.restoreAllMocks();
        document.body.replaceChildren();
    });

    function mount(rangeWidth: number, count: number, labels: Array<{ scroll: number; client: number }>) {
        const h = header(rangeWidth, 28, labels);
        const [revision, setRevision] = createSignal(0);
        const [tabCount, setTabCount] = createSignal(count);
        let steps!: ReturnType<typeof createStripSteps>;
        const dispose = createRoot((d) => {
            steps = createStripSteps({ strip: () => h.strip, tabCount, zoom: () => 1, revision });
            return d;
        });
        // onMount runs on the next microtask in a root.
        return { ...h, steps, dispose, setRevision, setTabCount };
    }

    it("sets the room the tabs may take, never under their floors", async () => {
        const m = mount(600, 4, []);
        await vi.advanceTimersByTimeAsync(40);
        expect(m.steps.room()).toBe(600 - 90 - 24);
        m.dispose();
        const tight = mount(120, 6, []);
        await vi.advanceTimersByTimeAsync(40);
        // 47 + 5 * 27 + the add button
        expect(tight.steps.room()).toBe(47 + 5 * 27 + 28);
        tight.dispose();
    });

    it("stays full when the room is short but no label is cut", async () => {
        const m = mount(300, 6, Array.from({ length: 6 }, () => ({ scroll: 40, client: 40 })));
        await vi.advanceTimersByTimeAsync(40);
        expect(m.steps.step()).toBe("full");
        m.dispose();
    });

    it("drops to the steps when labels are cut and each tab gets little", async () => {
        // room 186 - add 28 = 158 for 6 tabs: 26 each, under the icon step
        const m = mount(300, 6, Array.from({ length: 6 }, () => ({ scroll: 80, client: 10 })));
        await vi.advanceTimersByTimeAsync(40);
        expect(m.steps.step()).toBe("icon");
        m.dispose();
        const some = mount(480, 6, Array.from({ length: 6 }, () => ({ scroll: 80, client: 10 })));
        await vi.advanceTimersByTimeAsync(40);
        // 480 - 114 - 28 = 338 for 6: 56 each
        expect(some.steps.step()).toBe("icon-close");
        some.dispose();
    });

    it("measures again when the tab count changes", async () => {
        const m = mount(480, 6, Array.from({ length: 6 }, () => ({ scroll: 80, client: 10 })));
        await vi.advanceTimersByTimeAsync(40);
        expect(m.steps.step()).toBe("icon-close");
        m.setTabCount(2);
        await vi.advanceTimersByTimeAsync(40);
        // 338 for 2 tabs: 169 each
        expect(m.steps.step()).toBe("full");
        m.dispose();
    });
});
