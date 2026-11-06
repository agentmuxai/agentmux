// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PeekOverlay, pinned as a whole before it was split into hook groups: each scenario drives the panel through
 * a hover (mouse moves, the row's leave, the linger, the approach, the pointer on the panel, a zoomed pane,
 * a row that changes) and logs, in order, what the panel's style and attributes are, which rows the bridge and
 * the open-after-leave marker name, and the `autoUpdate` registrations and disposals. The logs are snapshots
 * recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { autoUpdate } from "@floating-ui/dom";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PeekOverlay } from "./PeekOverlay";
import { peekBridgeRow, peekPanelOpenAfterLeaveRow } from "./peek-bridge";

const T: string[] = [];
const log = (line: string) => T.push(line);
const SNAP = (name: string) => `./__snapshots__/PeekOverlay.behavior.${name}.txt`;

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(),
}));

let rowIds = new Map<Element, string>();
const nameOf = (el: Element | null) => (el ? (rowIds.get(el) ?? "?") : "-");

beforeEach(() => {
    T.length = 0;
    rowIds = new Map();
    vi.useFakeTimers();
    let n = 0;
    vi.mocked(autoUpdate).mockReset();
    vi.mocked(autoUpdate).mockImplementation(((_row: Element, _el: Element, update: () => void) => {
        const id = ++n;
        log(`autoUpdate #${id} registered (row ${nameOf(_row)})`);
        update();
        return () => log(`autoUpdate #${id} disposed`);
    }) as unknown as typeof autoUpdate);
    Object.defineProperty(window, "innerWidth", { value: 1400, configurable: true });
    Object.defineProperty(window, "innerHeight", { value: 1000, configurable: true });
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    document.body.innerHTML = "";
});

const rectOf = (el: Element, r: Partial<DOMRect>) =>
    vi.spyOn(el, "getBoundingClientRect").mockReturnValue({
        top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0, x: 0, y: 0, toJSON: () => {}, ...r,
    } as DOMRect);

const panel = () => document.querySelector(".agent-node-peek-overlay") as HTMLElement | null;

/** The panel, the bridge and the marker right now. */
function state(label: string) {
    const p = panel();
    log(
        `[${label}] panel=${p ? `{${p.getAttribute("style")}} outside=${p.hasAttribute("data-pane-overlay")} hidden=${p.getAttribute("data-window-tab-hidden")} class="${p.className}"` : "-"} bridge=${nameOf(peekBridgeRow())} afterLeave=${nameOf(peekPanelOpenAfterLeaveRow())}`
    );
}

function layout(name: string, rowRect: Partial<DOMRect> = {}, zoom?: string) {
    const container = document.createElement("div");
    container.style.overflowY = "auto";
    document.body.appendChild(container);
    const row = document.createElement("div");
    if (zoom) row.style.setProperty("--agent-pane-zoom", zoom);
    container.appendChild(row);
    rowIds.set(row, name);
    rectOf(container, { top: 100, bottom: 400 });
    rectOf(row, { top: 200, bottom: 230, left: 100, right: 800, width: 700, ...rowRect });
    return row;
}

function mount(row: HTMLElement, opts: { class?: string; shown?: boolean } = {}) {
    const [show, setShow] = createSignal(opts.shown ?? true);
    const [rowEl, setRowEl] = createSignal<HTMLElement | undefined>(row);
    render(() => (
        <PeekOverlay show={show()} rowEl={() => rowEl()} class={opts.class}>
            <span>peek content</span>
        </PeekOverlay>
    ));
    return { setShow, setRowEl };
}

const measure = (height: number, width = 300) =>
    rectOf(panel()!, { height, width, top: 230, bottom: 230 + height, left: 200, right: 200 + width });
const move = (row: Element, y: number) => {
    log(`row mousemove y=${y}`);
    row.dispatchEvent(new MouseEvent("mousemove", { clientY: y, bubbles: true }));
    vi.advanceTimersByTime(20);
};
const pointer = (x: number, y: number) => {
    log(`document mousemove ${x},${y}`);
    document.dispatchEvent(new MouseEvent("mousemove", { clientX: x, clientY: y }));
};
const advance = (ms: number) => {
    log(`advance ${ms}ms`);
    vi.advanceTimersByTime(ms);
};
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

describe("PeekOverlay behavior", () => {
    it("a panel that fits follows the pointer and closes at once", async () => {
        const row = layout("rowA");
        const { setShow } = mount(row);
        state("mounted");
        advance(20);
        measure(40);
        state("registered");
        move(row, 215);
        state("pointer at 215");
        move(row, 222);
        state("pointer at 222");
        setShow(false);
        state("show off");
        advance(2000);
        state("long after");
        await snapshot("fits");
    });

    it("a tall panel pins, lingers, is held by the pointer and closes after it leaves", async () => {
        const row = layout("rowA");
        const { setShow } = mount(row);
        advance(20);
        measure(5000);
        move(row, 215);
        state("tall, pointer at 215");
        move(row, 260);
        state("pointer moved: pinned, not following");
        setShow(false);
        state("show off: lingering");
        advance(100);
        state("100ms");
        pointer(300, 500);
        advance(100);
        state("approaching: re-armed");
        pointer(250, 400);
        advance(149);
        state("closer again, 149ms");
        advance(1);
        state("closed after the last linger");
        setShow(true);
        advance(20);
        measure(5000);
        move(row, 260);
        state("shown again, measured and moved");
        setShow(false);
        const p = panel()!;
        p.dispatchEvent(new MouseEvent("mouseenter"));
        advance(5000);
        state("the pointer is on the panel: held");
        p.dispatchEvent(new MouseEvent("mouseleave"));
        state("left the panel");
        advance(160);
        state("closed");
        await snapshot("tall");
    });

    it("an approach cannot keep it open past the cap, and a pointer moving away does not", async () => {
        const row = layout("rowA");
        const { setShow } = mount(row);
        advance(20);
        measure(5000);
        move(row, 215);
        setShow(false);
        let d = 1000;
        for (let i = 0; i < 12; i++) {
            d -= 60;
            pointer(500, d);
            advance(100);
        }
        state("after 12 approaches");
        advance(300);
        state("past the cap");
        setShow(true);
        advance(20);
        measure(5000);
        move(row, 215);
        setShow(false);
        pointer(900, 900);
        advance(100);
        pointer(1000, 1000);
        advance(100);
        state("moving away");
        await snapshot("approach");
    });

    it("reaches over the pane borders when wider than the row, and widens a thin tall panel", async () => {
        const row = layout("rowA", { left: 100, right: 400, width: 300 });
        mount(row, { class: "extra-class" });
        advance(20);
        measure(40, 900);
        move(row, 215);
        state("wider than the row");
        cleanup();
        log("-- next layout");
        const row2 = layout("rowB", { left: 100, right: 800, width: 700 });
        mount(row2);
        advance(20);
        measure(70, 120);
        move(row2, 215);
        state("thin and three lines tall");
        cleanup();
        log("-- next layout");
        const row3 = layout("rowC", { left: 100, right: 800, width: 700 });
        mount(row3);
        advance(20);
        measure(45, 120);
        move(row3, 215);
        state("two lines tall");
        await snapshot("wide");
    });

    it("compensates for the pane's zoom, and ignores a bad factor", async () => {
        for (const zoom of ["2", "0.5", "1", "abc", "-3", "0"]) {
            cleanup();
            document.body.innerHTML = "";
            const row = layout(`row-z${zoom}`, {}, zoom);
            mount(row);
            advance(20);
            measure(40);
            move(row, 215);
            state(`zoom ${zoom}`);
        }
        await snapshot("zoom");
    });

    it("hands the bridge and the marker over between rows, and cleans up on unmount", async () => {
        const rowA = layout("rowA");
        const rowB = layout("rowB");
        const { setShow, setRowEl } = mount(rowA);
        advance(20);
        measure(5000);
        move(rowA, 215);
        setShow(false);
        state("rowA lingering");
        setRowEl(rowB);
        advance(10);
        setShow(true);
        state("show back on, row switched");
        setShow(false);
        state("show off again with rowB");
        advance(2000);
        state("closed");
        setShow(true);
        advance(20);
        measure(5000);
        move(rowB, 215);
        setShow(false);
        state("lingering on rowB, about to unmount");
        cleanup();
        log(`after unmount: bridge=${nameOf(peekBridgeRow())} afterLeave=${nameOf(peekPanelOpenAfterLeaveRow())} panel=${panel() ? "present" : "-"}`);
        await snapshot("rows");
    });

    it("leaves nothing registered for a hover that ends before the first frame, and re-registers on the next", async () => {
        const row = layout("rowA");
        const { setShow } = mount(row);
        setShow(false);
        advance(500);
        state("left before the frame");
        setShow(true);
        advance(20);
        measure(40);
        state("shown again");
        move(row, 230);
        setShow(false);
        advance(100);
        state("hidden");
        await snapshot("race");
    });

    it("coalesces mouse moves into one frame, and drops a pending frame and a missing row cleanly", async () => {
        const row = layout("rowA");
        const { setRowEl } = mount(row);
        advance(20);
        measure(40);
        log("two moves in one frame");
        row.dispatchEvent(new MouseEvent("mousemove", { clientY: 210, bubbles: true }));
        row.dispatchEvent(new MouseEvent("mousemove", { clientY: 220, bubbles: true }));
        state("before the frame");
        advance(20);
        state("after the frame: the last y wins");
        log("a move, then unmount before its frame");
        row.dispatchEvent(new MouseEvent("mousemove", { clientY: 226, bubbles: true }));
        cleanup();
        advance(100);
        log(`after unmount: panel=${panel() ? "present" : "-"}`);
        document.body.innerHTML = "";
        const row2 = layout("rowB");
        const { setRowEl: setRow2 } = mount(row2);
        setRow2(undefined);
        advance(100);
        state("the row went away before the first frame");
        setRowEl(undefined);
        await snapshot("coalesce");
    });
});
