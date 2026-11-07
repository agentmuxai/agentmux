// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Characterisation tests for TypeAheadModal and its Suggestions list: what is drawn for flat and
 * grouped suggestions (icons, swatches, details, the current tick, the highlighted row and how it is
 * counted across groups), how each callback is wired (backdrop, keys, typing, selecting, the focus
 * handle), and how the modal is placed and sized from the pane and the anchor (the width and left
 * clamp, the top under the header, the height capped at 80% of the pane). They pin today's
 * behaviour (written against the unsplit component) so it can be split into named pieces without
 * changing it. Layout is stubbed (rects and computed styles by element), as is the resize observer
 * and the focus helper; the Input, the suggestions and the DOM are real. The real InputGroup takes
 * no ref, so the height effect never runs in the app; the tests give it one with `h.groupRef` to
 * pin the effect's arithmetic, and pin the app's case with it off.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    focusOnOpen: vi.fn<(el: HTMLElement, opts?: unknown) => void>(),
    groupRef: true,
    blockRect: { left: 100, width: 800, height: 600 },
    anchorRect: { left: 150, width: 40, height: 20 },
    headerHeight: 36,
    inputHeight: 30,
    itemHeight: 20,
    modalStyle: { paddingTop: "6px", paddingBottom: "6px", borderTopWidth: "1px", borderBottomWidth: "1px" },
    wrapperMarginTop: "8px",
    observers: [] as { cb: ResizeObserverCallback; observed: Element[]; disconnect: ReturnType<typeof vi.fn> }[],
}));

vi.mock("@/util/focusutil", () => ({ focusOnOpen: h.focusOnOpen }));
vi.mock("@/app/element/input", async () => {
    const real = await vi.importActual<typeof import("@/app/element/input")>("@/app/element/input");
    return {
        ...real,
        InputGroup: (props: { ref?: (el: HTMLDivElement) => void; children?: JSX.Element }) => (
            <div
                class="input-group"
                ref={(el) => {
                    if (h.groupRef) props.ref?.(el);
                }}
            >
                {props.children}
            </div>
        ),
    };
});

import { makeIconClass } from "@/util/util";
import { TypeAheadModal } from "./typeaheadmodal";

function rect(left: number, width: number, height: number): DOMRect {
    return { left, top: 0, width, height, right: left + width, bottom: height, x: left, y: 0 } as DOMRect;
}

class FakeResizeObserver {
    cb: ResizeObserverCallback;
    observed: Element[] = [];
    disconnect = vi.fn();
    constructor(cb: ResizeObserverCallback) {
        this.cb = cb;
        h.observers.push(this);
    }
    observe(el: Element) {
        this.observed.push(el);
    }
    unobserve() {}
}

function resize(width: number, height: number): void {
    h.observers[0].cb(
        [{ contentRect: { width, height } } as ResizeObserverEntry],
        h.observers[0] as unknown as ResizeObserver
    );
}

type Props = Parameters<typeof TypeAheadModal>[0];

type Fixture = {
    block: HTMLDivElement;
    anchor: HTMLDivElement;
    header: HTMLDivElement;
};

let fx: Fixture;

function makeFixture(): Fixture {
    const block = document.createElement("div");
    block.setAttribute("data-role", "block");
    const header = document.createElement("div");
    header.className = "block-frame-default-header";
    const anchor = document.createElement("div");
    anchor.setAttribute("data-role", "anchor");
    header.appendChild(anchor);
    block.appendChild(header);
    document.body.appendChild(block);
    return { block, anchor, header };
}

function mount(props: Partial<Props> = {}) {
    return render(() => (
        <TypeAheadModal
            anchorRef={{ current: fx.anchor }}
            blockRef={{ current: fx.block }}
            suggestions={[]}
            {...props}
        />
    ));
}

const flat = (...labels: string[]): SuggestionConnectionItem[] =>
    labels.map(
        (label) => ({ label, value: `v-${label}`, status: "connected", iconColor: "red" }) as SuggestionConnectionItem
    );

function modalEl(): HTMLElement {
    return fx.block.querySelector(".type-ahead-modal") as HTMLElement;
}

function wrapperEl(): HTMLElement {
    return fx.block.querySelector(".suggestions-wrapper") as HTMLElement;
}

function items(): HTMLElement[] {
    return Array.from(fx.block.querySelectorAll<HTMLElement>(".suggestion-item"));
}

beforeEach(() => {
    h.focusOnOpen.mockReset();
    h.observers.length = 0;
    h.groupRef = true;
    h.blockRect = { left: 100, width: 800, height: 600 };
    h.anchorRect = { left: 150, width: 40, height: 20 };
    h.headerHeight = 36;
    h.inputHeight = 30;
    h.itemHeight = 20;
    h.modalStyle = { paddingTop: "6px", paddingBottom: "6px", borderTopWidth: "1px", borderBottomWidth: "1px" };
    h.wrapperMarginTop = "8px";
    fx = makeFixture();
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
        const role = this.getAttribute("data-role");
        if (role === "block") return rect(h.blockRect.left, h.blockRect.width, h.blockRect.height);
        if (role === "anchor") return rect(h.anchorRect.left, h.anchorRect.width, h.anchorRect.height);
        if (this.classList.contains("block-frame-default-header")) return rect(0, 100, h.headerHeight);
        if (this.classList.contains("input-group")) return rect(0, 100, h.inputHeight);
        if (this.parentElement?.classList.contains("suggestions")) return rect(0, 100, h.itemHeight);
        return rect(0, 0, 0);
    });
    vi.spyOn(window, "getComputedStyle").mockImplementation(((el: Element) => {
        if (el.classList.contains("type-ahead-modal")) return h.modalStyle;
        if (el.classList.contains("suggestions-wrapper")) return { marginTop: h.wrapperMarginTop };
        return {};
    }) as unknown as typeof window.getComputedStyle);
});

afterEach(() => {
    cleanup();
    fx.block.remove();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

describe("Suggestions", () => {
    it("draws a flat item with its label and no icon, swatch, detail or tick", () => {
        mount({ suggestions: [{ label: "alpha", value: "a" } as SuggestionConnectionItem] });
        const list = fx.block.querySelector(".suggestions") as HTMLElement;
        expect(list).not.toBeNull();
        expect(items().length).toBe(1);
        const name = items()[0].querySelector(".typeahead-item-name") as HTMLElement;
        expect(name.className).toBe("typeahead-item-name ellipsis");
        expect(name.textContent).toBe("alpha");
        expect(items()[0].querySelector("i")).toBeNull();
        expect(items()[0].querySelector(".typeahead-swatch")).toBeNull();
        expect(items()[0].querySelector(".typeahead-detail")).toBeNull();
        expect(items()[0].querySelector(".typeahead-current-checkbox")).toBeNull();
    });

    it("draws a string icon as an icon element in the item's icon colour", () => {
        mount({
            suggestions: [{ label: "alpha", value: "a", icon: "server", iconColor: "red" } as SuggestionConnectionItem],
        });
        const icon = items()[0].querySelector(".typeahead-item-name i") as HTMLElement;
        expect(icon.className).toBe(makeIconClass("server", false));
        expect(icon.style.color).toBe("red");
    });

    it("draws an icon in inherited colour when the item has none, and for items that cannot carry one", () => {
        mount({
            suggestions: [
                { label: "a", value: "a", icon: "server" } as SuggestionConnectionItem,
                { label: "b", value: "b", icon: "server", iconColor: "" } as SuggestionConnectionItem,
            ],
        });
        const colours = items().map((it) => (it.querySelector("i") as HTMLElement).style.color);
        expect(colours).toEqual(["inherit", "inherit"]);
    });

    it("draws no icon for an empty icon name", () => {
        mount({ suggestions: [{ label: "a", value: "a", icon: "", iconColor: "red" } as SuggestionConnectionItem] });
        expect(items()[0].querySelector("i")).toBeNull();
        expect(items()[0].querySelector(".typeahead-item-name")?.textContent).toBe("a");
    });

    it("draws an element icon as it is", () => {
        mount({
            suggestions: [
                {
                    label: "alpha",
                    value: "a",
                    icon: (<b class="mine">!</b>) as JSX.Element,
                } as SuggestionConnectionItem,
            ],
        });
        expect(items()[0].querySelector(".typeahead-item-name b.mine")).not.toBeNull();
        expect(items()[0].querySelector("i")).toBeNull();
    });

    it("puts the swatch before the label and the detail after it", () => {
        mount({
            suggestions: [
                {
                    label: "alpha",
                    value: "a",
                    icon: "server",
                    iconColor: "red",
                    swatchColor: "#112233",
                    detail: "linux",
                } as SuggestionConnectionItem,
            ],
        });
        const name = items()[0].querySelector(".typeahead-item-name") as HTMLElement;
        expect(Array.from(name.childNodes).map((n) => (n as HTMLElement).className ?? n.textContent)).toEqual([
            makeIconClass("server", false),
            "typeahead-swatch",
            "alpha",
            "typeahead-detail",
        ]);
        expect((name.querySelector(".typeahead-swatch") as HTMLElement).style.background).toContain("rgb(17, 34, 51)");
        expect(name.querySelector(".typeahead-detail")?.textContent).toBe("linux");
    });

    it("leaves out an empty swatch colour and an empty detail", () => {
        mount({
            suggestions: [{ label: "a", value: "a", swatchColor: "", detail: "" } as SuggestionConnectionItem],
        });
        expect(items()[0].querySelector(".typeahead-swatch")).toBeNull();
        expect(items()[0].querySelector(".typeahead-detail")).toBeNull();
    });

    it("ticks the current item", () => {
        mount({
            suggestions: [
                { label: "a", value: "a", current: true } as SuggestionConnectionItem,
                { label: "b", value: "b", current: false } as SuggestionConnectionItem,
            ],
        });
        const tick = items()[0].querySelector("i.typeahead-current-checkbox") as HTMLElement;
        expect(tick.className).toBe(`${makeIconClass("check", false)} typeahead-current-checkbox`);
        expect(items()[1].querySelector(".typeahead-current-checkbox")).toBeNull();
    });

    it("highlights the row at the select index", () => {
        mount({ suggestions: flat("a", "b", "c"), selectIndex: 1 });
        expect(items().map((it) => it.className)).toEqual([
            "suggestion-item",
            "suggestion-item selected",
            "suggestion-item",
        ]);
    });

    it("highlights nothing without a select index", () => {
        mount({ suggestions: flat("a", "b") });
        expect(items().some((it) => it.classList.contains("selected"))).toBe(false);
    });

    it("draws a group's header and its items, counting rows across groups and loose items", () => {
        const grouped: SuggestionsType[] = [
            flat("loose")[0],
            { headerText: "SSH", items: flat("a", "b") },
            { headerText: "", items: flat("c") },
            { headerText: undefined, items: flat("d") },
        ];
        mount({ suggestions: grouped, selectIndex: 3 });
        const list = fx.block.querySelector(".suggestions") as HTMLElement;
        expect(list.children.length).toBe(4);
        expect(list.children[1].querySelector(".suggestion-header")?.textContent).toBe("SSH");
        expect(list.children[2].querySelector(".suggestion-header")).toBeNull();
        expect(list.children[3].querySelector(".suggestion-header")).toBeNull();
        expect(items().map((it) => it.textContent)).toEqual(["loose", "a", "b", "c", "d"]);
        expect(items().map((it) => it.classList.contains("selected"))).toEqual([false, false, false, true, false]);
    });

    it("draws a scope without a header key as an empty loose item", () => {
        mount({ suggestions: [{ items: flat("d") }] });
        const list = fx.block.querySelector(".suggestions") as HTMLElement;
        expect(items().length).toBe(1);
        expect(items()[0].textContent).toBe("");
        expect(list.querySelector(".suggestion-header")).toBeNull();
    });

    it("calls an item's own select with its value, not the modal's", () => {
        const own = vi.fn();
        const onSelect = vi.fn();
        mount({
            onSelect,
            suggestions: [{ label: "a", value: "va", onSelect: own } as unknown as SuggestionConnectionItem],
        });
        items()[0].click();
        expect(own).toHaveBeenCalledWith("va");
        expect(onSelect).not.toHaveBeenCalled();
    });

    it("calls the modal's select for an item whose own select is undefined", () => {
        const onSelect = vi.fn();
        mount({
            onSelect,
            suggestions: [{ label: "a", value: "va", onSelect: undefined } as unknown as SuggestionConnectionItem],
        });
        items()[0].click();
        expect(onSelect).toHaveBeenCalledWith("va");
    });

    it("calls the modal's select with the value for an item without its own", () => {
        const onSelect = vi.fn();
        mount({ onSelect, suggestions: flat("a", "b") });
        items()[1].click();
        expect(onSelect).toHaveBeenCalledTimes(1);
        expect(onSelect).toHaveBeenCalledWith("v-b");
    });

    it("selects a grouped item the same way", () => {
        const onSelect = vi.fn();
        mount({ onSelect, suggestions: [{ headerText: "SSH", items: flat("a") }] });
        items()[0].click();
        expect(onSelect).toHaveBeenCalledWith("v-a");
    });

    it("does nothing when an item without its own select is clicked and the modal has none", () => {
        mount({ suggestions: flat("a") });
        expect(() => items()[0].click()).not.toThrow();
    });

    it("keeps counting rows where it stopped when the suggestions change, so the highlight is lost", () => {
        const [list, setList] = createSignal<SuggestionsType[]>(flat("a", "b"));
        render(() => (
            <TypeAheadModal
                anchorRef={{ current: fx.anchor }}
                blockRef={{ current: fx.block }}
                suggestions={list()}
                selectIndex={0}
            />
        ));
        expect(items().map((it) => it.classList.contains("selected"))).toEqual([true, false]);
        setList(flat("x", "y", "z"));
        expect(items().map((it) => it.textContent)).toEqual(["x", "y", "z"]);
        expect(items().map((it) => it.classList.contains("selected"))).toEqual([false, false, false]);
    });
});

describe("TypeAheadModal frame", () => {
    it("draws nothing while the pane element is not there yet", () => {
        const { container } = mount({ blockRef: { current: null as unknown as HTMLDivElement } });
        expect(container.innerHTML).toBe("");
        expect(fx.block.querySelector(".type-ahead-modal-wrapper")).toBeNull();
    });

    it("throws a TypeError without a pane reference, and draws nothing", () => {
        const errors = vi.spyOn(console, "error").mockImplementation(() => {});
        expect(() => render(() => <TypeAheadModal anchorRef={{ current: fx.anchor }} />)).toThrow(TypeError);
        expect(fx.block.querySelector(".type-ahead-modal")).toBeNull();
        errors.mockRestore();
    });

    it("draws into the pane element: a wrapper holding the backdrop and the modal", () => {
        const { container } = mount();
        expect(container.innerHTML).toBe("");
        const wrapper = fx.block.querySelector(".type-ahead-modal-wrapper") as HTMLElement;
        expect(wrapper.parentElement?.parentElement).toBe(fx.block);
        expect(Array.from(wrapper.children).map((c) => c.className)).toEqual([
            "type-ahead-modal-backdrop",
            "type-ahead-modal",
        ]);
    });

    it("calls the backdrop handler on a backdrop click, and copes with none", () => {
        const onClickBackdrop = vi.fn();
        mount({ onClickBackdrop });
        (fx.block.querySelector(".type-ahead-modal-backdrop") as HTMLElement).click();
        expect(onClickBackdrop).toHaveBeenCalledTimes(1);
        cleanup();
        mount();
        expect(() => (fx.block.querySelector(".type-ahead-modal-backdrop") as HTMLElement).click()).not.toThrow();
    });

    it("passes a key press in the modal on to the key handler, and copes with none", () => {
        const onKeyDown = vi.fn();
        mount({ onKeyDown });
        const input = fx.block.querySelector("input") as HTMLInputElement;
        fireEvent.keyDown(input, { key: "ArrowDown" });
        expect(onKeyDown).toHaveBeenCalledTimes(1);
        expect((onKeyDown.mock.calls[0][0] as KeyboardEvent).key).toBe("ArrowDown");
        cleanup();
        mount();
        expect(() => fireEvent.keyDown(fx.block.querySelector("input") as HTMLElement, { key: "a" })).not.toThrow();
    });

    it("adds the caller's class, and has-suggestions only with suggestions", () => {
        mount({ class: "mine", suggestions: [] });
        expect(modalEl().className).toBe("type-ahead-modal mine");
        cleanup();
        mount({ class: "mine", suggestions: flat("a") });
        expect(modalEl().className).toBe("type-ahead-modal mine has-suggestions");
        cleanup();
        mount({ suggestions: undefined });
        expect(modalEl().className).toBe("type-ahead-modal");
    });

    it("reacts to suggestions arriving", () => {
        const [list, setList] = createSignal<SuggestionsType[]>([]);
        render(() => (
            <TypeAheadModal anchorRef={{ current: fx.anchor }} blockRef={{ current: fx.block }} suggestions={list()} />
        ));
        expect(modalEl().classList.contains("has-suggestions")).toBe(false);
        expect(fx.block.querySelector(".suggestions")).toBeNull();
        setList(flat("a"));
        expect(modalEl().classList.contains("has-suggestions")).toBe(true);
        expect(fx.block.querySelector(".suggestions")).not.toBeNull();
    });
});

describe("TypeAheadModal input", () => {
    it("is a labelled search input with a magnifier", () => {
        mount({ label: "Connect to", value: "box" });
        const input = fx.block.querySelector("input") as HTMLInputElement;
        expect(input.placeholder).toBe("Connect to");
        expect(input.value).toBe("box");
        expect(input.getAttribute("aria-label")).toBe("Connect to");
        const right = fx.block.querySelector(".input-right-element") as HTMLElement;
        expect(right.querySelector("i")?.className).toBe("fa-solid fa-magnifying-glass");
        expect(fx.block.querySelector(".input-group")?.contains(input)).toBe(true);
    });

    it("shows the value it is given as it changes", () => {
        const [value, setValue] = createSignal("a");
        render(() => (
            <TypeAheadModal anchorRef={{ current: fx.anchor }} blockRef={{ current: fx.block }} value={value()} />
        ));
        const input = fx.block.querySelector("input") as HTMLInputElement;
        expect(input.value).toBe("a");
        setValue("abc");
        expect(input.value).toBe("abc");
    });

    it("reports typing to the change handler, and copes with none", () => {
        const onChange = vi.fn();
        mount({ onChange, value: "" });
        const input = fx.block.querySelector("input") as HTMLInputElement;
        input.value = "ssh";
        fireEvent.input(input);
        expect(onChange).toHaveBeenCalledWith("ssh");
        cleanup();
        mount({ value: "" });
        const again = fx.block.querySelector("input") as HTMLInputElement;
        again.value = "x";
        expect(() => fireEvent.input(again)).not.toThrow();
    });

    it("focuses itself when asked to, and not otherwise", () => {
        mount({ autoFocus: true });
        expect(h.focusOnOpen).toHaveBeenCalledTimes(1);
        expect(h.focusOnOpen.mock.calls[0][0]).toBe(fx.block.querySelector("input"));
        cleanup();
        h.focusOnOpen.mockReset();
        mount({ autoFocus: false });
        expect(h.focusOnOpen).not.toHaveBeenCalled();
    });

    it("hands the caller a function that focuses the input and says it did", () => {
        const giveFocusRef = { current: (() => false) as () => boolean };
        mount({ giveFocusRef });
        const input = fx.block.querySelector("input") as HTMLInputElement;
        expect(document.activeElement).not.toBe(input);
        expect(giveFocusRef.current()).toBe(true);
        expect(document.activeElement).toBe(input);
    });

    it("leaves the focus handle alone when none is given", () => {
        expect(() => mount()).not.toThrow();
    });
});

describe("TypeAheadModal suggestions area", () => {
    it("is a scrolling area with no margin and no list while there are no suggestions", () => {
        mount({ suggestions: [] });
        expect(wrapperEl().style.marginTop).toBe("0px");
        expect(wrapperEl().style.overflowY).toBe("auto");
        expect(wrapperEl().children.length).toBe(0);
    });

    it("opens a margin and the list when there are suggestions", () => {
        mount({ suggestions: flat("a", "b"), selectIndex: 1 });
        expect(wrapperEl().style.marginTop).toBe("8px");
        expect(wrapperEl().children.length).toBe(1);
        expect(wrapperEl().firstElementChild?.className).toBe("suggestions");
        expect(items()[1].classList.contains("selected")).toBe(true);
    });
});

describe("TypeAheadModal placement", () => {
    it("observes the pane for size changes", () => {
        mount();
        expect(h.observers.length).toBe(1);
        expect(h.observers[0].observed).toEqual([fx.block]);
    });

    it("observes nothing for a pane that is not there", () => {
        mount({ blockRef: { current: null as unknown as HTMLDivElement } });
        expect(h.observers.length).toBe(0);
    });

    it("does not disconnect the observer when it goes away", () => {
        const { unmount } = mount();
        unmount();
        expect(h.observers[0].disconnect).not.toHaveBeenCalled();
    });

    it("places the modal under the anchor, 300 px wide", () => {
        mount();
        expect(modalEl().style.width).toBe("300px");
        expect(modalEl().style.left).toBe("50px");
    });

    it("narrows the modal to the pane's width less a gap of 20 px each side", () => {
        h.blockRect = { left: 100, width: 250, height: 600 };
        mount();
        expect(modalEl().style.width).toBe("210px");
    });

    it("keeps a full 300 px modal in a pane just wide enough", () => {
        h.blockRect = { left: 100, width: 340, height: 600 };
        h.anchorRect = { left: 120, width: 40, height: 20 };
        mount();
        expect(modalEl().style.width).toBe("300px");
        expect(modalEl().style.left).toBe("20px");
    });

    it("pulls the modal back from the pane's right edge, to 16 px short of it", () => {
        h.anchorRect = { left: 800, width: 40, height: 20 };
        mount();
        expect(modalEl().style.left).toBe("484px");
    });

    it("keeps the modal 20 px in from the pane's left edge", () => {
        h.anchorRect = { left: 105, width: 40, height: 20 };
        mount();
        expect(modalEl().style.left).toBe("20px");
    });

    it("places the modal by the pane when the anchor is not on screen", () => {
        mount({ anchorRef: { current: null as unknown as HTMLElement } });
        expect(modalEl().style.left).toBe("20px");
        expect(modalEl().style.width).toBe("300px");
    });

    it("places the modal again when the pane's width changes", () => {
        mount();
        h.blockRect = { left: 100, width: 250, height: 600 };
        resize(250, 500);
        expect(modalEl().style.width).toBe("210px");
    });

    it("places the modal again only when the pane's width changes, not its height", () => {
        mount();
        resize(800, 600);
        h.blockRect = { left: 100, width: 250, height: 600 };
        resize(800, 400);
        expect(modalEl().style.width).toBe("300px");
        resize(250, 400);
        expect(modalEl().style.width).toBe("210px");
    });

    it("sits under the anchor's header", () => {
        mount();
        expect(modalEl().style.top).toBe("36px");
    });

    it("sets no top without a header around the anchor", () => {
        const loose = document.createElement("div");
        fx.block.appendChild(loose);
        mount({ anchorRef: { current: loose } });
        expect(modalEl().style.top).toBe("");
    });

    it("sets no top without an anchor", () => {
        mount({ anchorRef: { current: null as unknown as HTMLElement } });
        expect(modalEl().style.top).toBe("");
    });
});

describe("TypeAheadModal height", () => {
    it("is not set while the input group takes no ref (as in the app)", () => {
        h.groupRef = false;
        mount({ suggestions: flat("a", "b") });
        resize(800, 600);
        expect(modalEl().style.height).toBe("");
        expect(wrapperEl().style.height).toBe("");
    });

    it("is 0 before the pane is measured (80% of nothing), and the list area gets no negative height", () => {
        mount({ suggestions: flat("a", "b") });
        expect(modalEl().style.height).toBe("0px");
        expect(wrapperEl().style.height).toBe("");
    });

    it("fits the input and the suggestions plus padding, border and margin when they fit", () => {
        mount({ suggestions: flat("a", "b", "c") });
        resize(800, 600);
        // 6 + 6 padding, 1 + 1 border, 30 input, 3 x 20 rows, 8 margin
        expect(modalEl().style.height).toBe("112px");
        expect(wrapperEl().style.height).toBe("60px");
    });

    it("stops at 80% of the pane when the suggestions would not fit", () => {
        mount({ suggestions: flat("a", "b", "c", "d", "e", "f", "g", "h", "i", "j") });
        resize(800, 200);
        expect(modalEl().style.height).toBe("160px");
        expect(wrapperEl().style.height).toBe("108px");
    });

    it("counts each child of the list once: a group, however many rows it holds, is one", () => {
        mount({ suggestions: [{ headerText: "SSH", items: flat("a", "b") }] });
        resize(800, 600);
        expect(modalEl().style.height).toBe("72px");
    });

    it("reads the padding, border and margin as numbers, taking a missing one as 0", () => {
        h.modalStyle = { paddingTop: "", paddingBottom: "4px", borderTopWidth: "abc", borderBottomWidth: "2px" };
        h.wrapperMarginTop = "";
        mount({ suggestions: flat("a") });
        resize(800, 600);
        // 0 + 4 padding, 0 + 2 border, 30 input, 20 row, 0 margin
        expect(modalEl().style.height).toBe("56px");
        expect(wrapperEl().style.height).toBe("20px");
    });

    it("takes a missing bottom padding and bottom border as 0 too", () => {
        h.modalStyle = { paddingTop: "3px", paddingBottom: "", borderTopWidth: "1px", borderBottomWidth: "abc" };
        mount({ suggestions: flat("a") });
        resize(800, 600);
        // 3 + 0 padding, 1 + 0 border, 30 input, 20 row, 8 margin
        expect(modalEl().style.height).toBe("62px");
        expect(wrapperEl().style.height).toBe("20px");
    });

    it("is sized again as the pane's height changes", () => {
        mount({ suggestions: flat("a") });
        resize(800, 600);
        const first = modalEl().style.height;
        resize(800, 50);
        expect(first).toBe("72px");
        expect(modalEl().style.height).toBe("40px");
    });

    it("is not set without suggestions", () => {
        mount({ suggestions: [] });
        resize(800, 600);
        expect(modalEl().style.height).toBe("");
    });
});
