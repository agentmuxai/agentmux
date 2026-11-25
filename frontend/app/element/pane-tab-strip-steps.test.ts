// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { activeTabNeeds, createStripSteps, stripRoom } from "./pane-tab-strip-steps";

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

    it.each([2, 0.5])("counts a neighbour in the parent's own pixels when the row is drawn at a zoom of %s", (zoom) => {
        const parent = document.createElement("div");
        const toggle = document.createElement("button");
        const strip = document.createElement("div");
        parent.append(toggle, strip);
        document.body.append(parent);
        // 500 layout px, drawn at `zoom` times that; the 28 px toggle is drawn at `zoom` times its layout width too.
        Object.defineProperty(parent, "clientWidth", { value: 500 });
        parent.getBoundingClientRect = () => ({ width: 500 * zoom }) as DOMRect;
        toggle.getBoundingClientRect = () => ({ width: 28 * zoom }) as DOMRect;
        Object.defineProperty(toggle, "offsetParent", { value: document.body });
        // the toggle and one gap of 8, both in layout px
        expect(stripRoom(strip)).toBe(500 - 28 - 8);
    });

    it("is the parent's width less what else is in its row, for a strip beside other items (the Editor's file tree toggle)", () => {
        const parent = document.createElement("div");
        const toggle = document.createElement("button");
        const hidden = document.createElement("div");
        const strip = document.createElement("div");
        parent.append(toggle, hidden, strip);
        document.body.append(parent);
        Object.defineProperty(parent, "clientWidth", { value: 500 });
        toggle.getBoundingClientRect = () => ({ width: 28 }) as DOMRect;
        Object.defineProperty(toggle, "offsetParent", { value: document.body });
        // Not shown: takes no room (its offsetParent is null, as for `display: none`).
        hidden.getBoundingClientRect = () => ({ width: 99 }) as DOMRect;
        // the toggle and one gap of 8
        expect(stripRoom(strip)).toBe(500 - 28 - 8);
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

    it("keeps a wider floor for tabs that have no icon, so the strip scrolls before they become slivers", async () => {
        const m = mount(120, 6, []);
        const tab = document.createElement("div");
        tab.className = "pane-tab doc-tab--no-icon";
        m.strip.append(tab);
        m.setRevision(1);
        await vi.advanceTimersByTimeAsync(40);
        // the selected tab's 76 + 5 * 56 + the add button
        expect(m.steps.room()).toBe(76 + 5 * 56 + 28);
        m.dispose();
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

    it("measures again when an item joins its row outside a header (the Editor's file tree toggle)", async () => {
        const parent = document.createElement("div");
        const strip = document.createElement("div");
        parent.append(strip);
        document.body.append(parent);
        Object.defineProperty(parent, "clientWidth", { value: 500 });
        let steps!: ReturnType<typeof createStripSteps>;
        const dispose = createRoot((d) => {
            steps = createStripSteps({ strip: () => strip, tabCount: () => 2, zoom: () => 1, revision: () => 0 });
            return d;
        });
        await vi.advanceTimersByTimeAsync(40);
        expect(steps.room()).toBe(500);
        // The tree is hidden: the toggle that shows it leads the row, and takes its width and a gap.
        const toggle = document.createElement("button");
        toggle.getBoundingClientRect = () => ({ width: 28 }) as DOMRect;
        Object.defineProperty(toggle, "offsetParent", { value: document.body });
        parent.prepend(toggle);
        await vi.advanceTimersByTimeAsync(40);
        expect(steps.room()).toBe(500 - 28 - 8);
        toggle.remove();
        await vi.advanceTimersByTimeAsync(40);
        expect(steps.room()).toBe(500);
        dispose();
    });

    it("holds its step while a tab is being renamed, and looks again when the input goes", async () => {
        const m = mount(480, 6, Array.from({ length: 6 }, () => ({ scroll: 80, client: 10 })));
        await vi.advanceTimersByTimeAsync(40);
        expect(m.steps.step()).toBe("icon-close");
        const input = document.createElement("input");
        input.className = "pane-tab-label pane-tab-rename-input";
        m.strip.append(input);
        m.setTabCount(2);
        await vi.advanceTimersByTimeAsync(40);
        expect(m.steps.step()).toBe("icon-close");
        input.remove();
        m.setRevision(1);
        await vi.advanceTimersByTimeAsync(40);
        expect(m.steps.step()).toBe("full");
        m.dispose();
    });
});

/** A strip with the selected tab's pill: its items stand where they would in a layout (`at`: left, width), the title between them. */
function selectedPill(strip: HTMLElement, items: Array<{ cls: string; left: number; width: number }>, pill = { left: 0, width: 47 }) {
    const el = document.createElement("div");
    el.className = "pane-tab pane-tab--active";
    const box = (left: number, width: number) => ({ left, width, right: left + width }) as DOMRect;
    el.getBoundingClientRect = () => box(pill.left, pill.width);
    Object.defineProperty(el, "offsetParent", { value: document.body });
    for (const it of items) {
        const child = document.createElement("span");
        child.className = it.cls;
        child.getBoundingClientRect = () => box(it.left, it.width);
        Object.defineProperty(child, "offsetParent", { value: document.body });
        el.append(child);
    }
    strip.prepend(el);
    return el;
}

describe("activeTabNeeds", () => {
    beforeEach(() => {
        vi.spyOn(window, "getComputedStyle").mockImplementation(() => ({ position: "static" }) as CSSStyleDeclaration);
    });
    afterEach(() => {
        vi.restoreAllMocks();
        document.body.replaceChildren();
    });

    // An agent's tab squeezed to 47: icon at 8, the title squeezed to nothing, six buttons of 22 and a gap of 2, the close button.
    const squeezed = [
        { cls: "pane-tab-icon", left: 8, width: 16 },
        { cls: "pane-tab-label", left: 28, width: 0 },
        { cls: "pane-tab-actions", left: 32, width: 142 },
        { cls: "pane-tab-close", left: 178, width: 16 },
    ];

    it("reads what is not the title and how long the title is, from where the items stand, so a squeezed tab says the same as a whole one", () => {
        const { strip } = header(600, 28, []);
        const pill = selectedPill(strip, squeezed);
        const label = pill.querySelector<HTMLElement>(".pane-tab-label")!;
        Object.defineProperty(label, "scrollWidth", { value: 160 });
        Object.defineProperty(label, "clientWidth", { value: 0 });
        const squeezedNeeds = activeTabNeeds(strip, 1)!;
        // 194 - 8 = 186 between the first item's left and the last one's right, plus the 8 of padding and 5 at the right.
        expect(squeezedNeeds.fixed).toBe(186 + 8 + 5);
        expect(squeezedNeeds.label).toBe(160);
        // The same tab with its title showing: 120 wide, the buttons and the close button pushed along by it.
        pill.replaceChildren();
        const whole = selectedPill(strip, [
            { cls: "pane-tab-icon", left: 8, width: 16 },
            { cls: "pane-tab-label", left: 28, width: 120 },
            { cls: "pane-tab-actions", left: 152, width: 142 },
            { cls: "pane-tab-close", left: 298, width: 16 },
        ], { left: 0, width: 320 });
        whole.className = "pane-tab pane-tab--active";
        pill.remove();
        const label2 = whole.querySelector<HTMLElement>(".pane-tab-label")!;
        Object.defineProperty(label2, "scrollWidth", { value: 160 });
        Object.defineProperty(label2, "clientWidth", { value: 120 });
        const wholeNeeds = activeTabNeeds(strip, 1)!;
        expect(wholeNeeds.fixed).toBe(squeezedNeeds.fixed);
        // The title is as long as its content, scaled as its box is (a box that shows 120 of 160 is not cut by the scale).
        expect(wholeNeeds.label).toBe(160);
    });

    it("is in unzoomed px at a strip zoom", () => {
        const { strip } = header(600, 28, []);
        const pill = selectedPill(strip, squeezed.map((it) => ({ ...it, left: it.left * 2, width: it.width * 2 })), { left: 0, width: 94 });
        const label = pill.querySelector<HTMLElement>(".pane-tab-label")!;
        Object.defineProperty(label, "scrollWidth", { value: 160 });
        Object.defineProperty(label, "clientWidth", { value: 0 });
        expect(activeTabNeeds(strip, 2)!.fixed).toBe(186 + 8 + 5);
    });

    it("is nothing with no selected tab", () => {
        const { strip } = header(600, 28, [{ scroll: 40, client: 40 }]);
        expect(activeTabNeeds(strip, 1)).toBeNull();
    });
});

describe("a strip whose selected tab carries buttons", () => {
    beforeEach(() => {
        stubStyle();
        vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "setTimeout", "clearTimeout"] });
    });
    afterEach(() => {
        vi.useRealTimers();
        vi.restoreAllMocks();
        document.body.replaceChildren();
    });

    function mountWith(rangeWidth: number, count: number) {
        const h = header(rangeWidth, 28, []);
        const pill = selectedPill(h.strip, [
            { cls: "pane-tab-icon", left: 8, width: 16 },
            { cls: "pane-tab-label", left: 28, width: 0 },
            { cls: "pane-tab-actions", left: 32, width: 142 },
            { cls: "pane-tab-close", left: 178, width: 16 },
        ]);
        const label = pill.querySelector<HTMLElement>(".pane-tab-label")!;
        Object.defineProperty(label, "scrollWidth", { value: 160 });
        Object.defineProperty(label, "clientWidth", { value: 0 });
        // Another tab's label, cut, so the strip needs a step.
        const other = document.createElement("span");
        other.className = "pane-tab-label";
        Object.defineProperty(other, "scrollWidth", { value: 200 });
        Object.defineProperty(other, "clientWidth", { value: 20 });
        h.strip.append(other);
        let steps!: ReturnType<typeof createStripSteps>;
        const dispose = createRoot((d) => {
            steps = createStripSteps({ strip: () => h.strip, tabCount: () => count, zoom: () => 1, revision: () => 0 });
            return d;
        });
        return { ...h, steps, dispose };
    }

    it("writes the selected tab's floor on the strip: its buttons whole and the start of its title", async () => {
        const m = mountWith(600, 5);
        await vi.advanceTimersByTimeAsync(40);
        // fixed 199 (the items' span, padding), plus the 56 of title
        expect(m.strip.style.getPropertyValue("--pane-tab-active-floor")).toBe("255px");
        m.dispose();
    });

    it("steps the other tabs by what the selected tab leaves them, not by an equal share", async () => {
        // 600 - 114 - 28 = 458 for five tabs: an equal share is 91 (full). The selected tab asks 199 + 160, so the others have 25.
        const m = mountWith(600, 5);
        await vi.advanceTimersByTimeAsync(40);
        expect(m.steps.step()).toBe("icon");
        m.dispose();
    });

    it("does not let the strip shrink past the selected tab's floor and the others'", async () => {
        const m = mountWith(120, 5);
        await vi.advanceTimersByTimeAsync(40);
        // 255 + 4 * 27 + the add button
        expect(m.steps.room()).toBe(255 + 4 * 27 + 28);
        m.dispose();
    });
});
