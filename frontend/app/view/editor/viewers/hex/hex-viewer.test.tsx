// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The hex viewer on screen: the rows it draws from a file read in parts, the
 * cursor and selection by key and by pointer, go to an offset, the row
 * width, copy, the data inspector, and a read that fails.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CHUNK } from "./hex-chunks";
import { ROW_H } from "./hex-layout";
import { HexViewer } from "./hex-viewer";
import { forgetHexMemory } from "./hex-viewer-state";

const fetchMediaRange = vi.hoisted(() => vi.fn());
const writeText = vi.hoisted(() => vi.fn(async (_text: string) => {}));
const pushNotification = vi.hoisted(() => vi.fn());

vi.mock("@/app/element/local-media", async (actual) => ({ ...(await actual<object>()), fetchMediaRange }));
vi.mock("@/util/clipboard", () => ({ writeText }));
vi.mock("@/app/store/global", () => ({ pushNotification }));

/** 200,000 bytes: byte n is n mod 251, so every offset's byte is known. */
const FILE = Uint8Array.from({ length: 200_000 }, (_, i) => i % 251);
const VIEW_ROWS = 10;

function serve(bytes: Uint8Array) {
    fetchMediaRange.mockImplementation(async (_path: string, start: number, end: number) => ({
        blob: { arrayBuffer: async () => bytes.slice(start, end + 1).buffer } as unknown as Blob,
        total: bytes.length,
    }));
}

async function settle() {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
}

async function mount(props: { size?: number; path?: string; memoryKey?: string; revision?: number } = {}) {
    const view = render(() => (
        <HexViewer
            path={props.path ?? "C:/bin/a.bin"}
            revision={props.revision ?? 0}
            size={props.size ?? FILE.length}
            memoryKey={props.memoryKey ?? "C:/bin/a.bin"}
            bar={{ viewer: "hex", options: ["hex", "info"], onReopenWith: () => {} }}
        />
    ));
    await settle();
    return view;
}

const grid = () => screen.getByRole("grid");
const rowTexts = () => Array.from(document.querySelectorAll(".hex-rows .hex-row")).map((r) => r.querySelector(".hex-offset")!.textContent);
const cell = (offset: number, column: "hex" | "text" = "hex") => document.querySelector<HTMLElement>(`.hex-rows [data-c="${column}"] [data-o="${offset}"]`)!;
const selected = () => Array.from(document.querySelectorAll('.hex-rows [data-c="hex"] .hex-sel')).map((c) => Number(c.getAttribute("data-o")));
const cursorAt = () => Number(document.querySelector('.hex-rows [data-c="hex"] .hex-cur')?.getAttribute("data-o"));
const facts = () => Array.from(document.querySelectorAll(".editor-viewer-bar-facts span")).map((s) => s.textContent);
const key = (k: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(grid(), { key: k, ...init });

describe("the hex viewer", () => {
    let clientHeight: PropertyDescriptor | undefined;

    beforeEach(() => {
        forgetHexMemory();
        fetchMediaRange.mockReset();
        writeText.mockClear();
        pushNotification.mockClear();
        serve(FILE);
        // A view ten rows tall under the header row.
        clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
        Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => ROW_H * (VIEW_ROWS + 1) });
    });

    afterEach(() => {
        cleanup();
        if (clientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
    });

    it("draws only the rows on screen, each an offset, its bytes in hex and their text", async () => {
        await mount();
        expect(rowTexts()).toEqual(Array.from({ length: VIEW_ROWS + 2 }, (_, i) => (i * 16).toString(16).toUpperCase().padStart(8, "0")));
        expect(cell(0).textContent).toBe("00");
        expect(cell(0x41).textContent).toBe("41");
        expect(cell(0x41, "text").textContent).toBe("A");
        expect(cell(0x0a, "text").textContent).toBe(".");
        // One part read for the first rows, not the file.
        expect(fetchMediaRange).toHaveBeenCalledTimes(1);
        expect(fetchMediaRange.mock.calls[0].slice(1, 3)).toEqual([0, CHUNK - 1]);
        expect(grid().getAttribute("aria-rowcount")).toBe(String(12_500 + 1));
        expect(facts()).toEqual(["Offset 0x0 (0)", "195 KB"]);
    });

    it("says so when the file is empty, and reads nothing", async () => {
        await mount({ size: 0 });
        expect(screen.getByText("This file is empty.")).toBeTruthy();
        expect(screen.queryByRole("grid")).toBeNull();
        expect(fetchMediaRange).not.toHaveBeenCalled();
    });

    it("moves the cursor with the keys, extends the selection with Shift, and scrolls to keep it on screen", async () => {
        await mount();
        key("ArrowRight");
        key("ArrowDown");
        expect(cursorAt()).toBe(17);
        expect(selected()).toEqual([17]);
        key("ArrowRight", { shiftKey: true });
        key("ArrowRight", { shiftKey: true });
        expect(selected()).toEqual([17, 18, 19]);
        expect(facts()).toContain("3 bytes selected");
        // A plain move drops the selection.
        key("ArrowLeft");
        expect(selected()).toEqual([18]);
        key("PageDown");
        key("PageDown");
        expect(cursorAt()).toBe(18 + 2 * VIEW_ROWS * 16);
        expect(rowTexts()[0]).toBe("000000C0");
        key("Home", { ctrlKey: true });
        expect(cursorAt()).toBe(0);
        expect(rowTexts()[0]).toBe("00000000");
    });

    it("goes to the last byte with Ctrl+End, reading the part that holds it", async () => {
        await mount();
        key("End", { ctrlKey: true });
        await settle();
        const last = FILE.length - 1;
        expect(cursorAt()).toBe(last);
        expect(cell(last).textContent).toBe((last % 251).toString(16).toUpperCase().padStart(2, "0"));
        expect(fetchMediaRange.mock.calls.at(-1)!.slice(1, 3)).toEqual([CHUNK * 3, last]);
        // The last row is short: no cells past the file's end.
        expect(document.querySelectorAll(".hex-rows .hex-row:last-child [data-c='hex'] [data-o]").length).toBe(FILE.length % 16 || 16);
    });

    it("leaves a modified arrow to the app", async () => {
        await mount();
        const e = new KeyboardEvent("keydown", { key: "ArrowRight", ctrlKey: true, bubbles: true, cancelable: true });
        grid().dispatchEvent(e);
        expect(e.defaultPrevented).toBe(false);
        expect(cursorAt()).toBe(0);
    });

    it("puts the cursor where a byte is pressed, and selects to where a drag or a Shift press goes", async () => {
        await mount();
        fireEvent.pointerDown(cell(4));
        expect(cursorAt()).toBe(4);
        fireEvent.pointerMove(cell(20));
        expect(selected()).toEqual(Array.from({ length: 17 }, (_, i) => 4 + i));
        fireEvent.pointerUp(window);
        // No drag any more: a move over another byte changes nothing.
        fireEvent.pointerMove(cell(40));
        expect(cursorAt()).toBe(20);
        fireEvent.pointerDown(cell(2), { shiftKey: true });
        expect(selected()).toEqual([2, 3, 4]);
        fireEvent.pointerUp(window);
        // A press in the text column makes it the column the keys act on.
        fireEvent.pointerDown(cell(7, "text"));
        expect(grid().getAttribute("data-column")).toBe("text");
        expect(cursorAt()).toBe(7);
    });

    it("moves between the two columns with Tab, and out of the grid past them", async () => {
        await mount();
        const tab = (shiftKey: boolean) => {
            const e = new KeyboardEvent("keydown", { key: "Tab", shiftKey, bubbles: true, cancelable: true });
            grid().dispatchEvent(e);
            return e.defaultPrevented;
        };
        expect(tab(true)).toBe(false);
        expect(tab(false)).toBe(true);
        expect(grid().getAttribute("data-column")).toBe("text");
        expect(tab(false)).toBe(false);
        expect(tab(true)).toBe(true);
        expect(grid().getAttribute("data-column")).toBe("hex");
    });

    it("goes to a typed offset, decimal or hex, and marks one outside the file", async () => {
        await mount();
        const box = screen.getByLabelText("Go to offset") as HTMLInputElement;
        expect(box.value).toBe("0x0");
        key("g", { ctrlKey: true });
        expect(document.activeElement).toBe(box);
        fireEvent.input(box, { target: { value: "0x1F40" } });
        fireEvent.keyDown(box, { key: "Enter" });
        await settle();
        expect(cursorAt()).toBe(0x1f40);
        // Two rows of context above it.
        expect(rowTexts()[0]).toBe("00001F20");
        expect(document.activeElement).toBe(grid());
        fireEvent.input(box, { target: { value: "999999999" } });
        fireEvent.keyDown(box, { key: "Enter" });
        expect(box.getAttribute("aria-invalid")).toBe("true");
        expect(cursorAt()).toBe(0x1f40);
        fireEvent.input(box, { target: { value: "5" } });
        expect(box.getAttribute("aria-invalid")).toBe("false");
    });

    it("shows 8 or 32 bytes to a row, keeping the cursor on screen", async () => {
        await mount();
        key("ArrowDown");
        fireEvent.click(screen.getByRole("radio", { name: "32" }));
        expect(rowTexts().slice(0, 2)).toEqual(["00000000", "00000020"]);
        expect(document.querySelectorAll(".hex-rows .hex-row:first-child [data-c='hex'] [data-o]").length).toBe(32);
        expect(cursorAt()).toBe(16);
        fireEvent.click(screen.getByRole("radio", { name: "8" }));
        expect(rowTexts().slice(0, 3)).toEqual(["00000000", "00000008", "00000010"]);
        expect(grid().getAttribute("aria-rowcount")).toBe(String(25_000 + 1));
    });

    it("copies the selection as hex with Ctrl+C in the hex column and as text in the text column", async () => {
        await mount();
        fireEvent.pointerDown(cell(0x41));
        fireEvent.pointerUp(window);
        key("ArrowRight", { shiftKey: true });
        key("c", { ctrlKey: true });
        await settle();
        expect(writeText).toHaveBeenLastCalledWith("41 42");
        fireEvent.pointerDown(cell(0x41, "text"));
        fireEvent.pointerUp(window);
        key("ArrowRight", { shiftKey: true });
        key("c", { ctrlKey: true });
        await settle();
        expect(writeText).toHaveBeenLastCalledWith("AB");
    });

    it("refuses a copy of more than a megabyte, and says why", async () => {
        const big = new Uint8Array(1_048_577 + 16);
        serve(big);
        await mount({ size: big.length });
        key("a", { ctrlKey: true });
        key("c", { ctrlKey: true });
        await settle();
        expect(writeText).not.toHaveBeenCalled();
        expect(pushNotification).toHaveBeenCalledOnce();
        expect(pushNotification.mock.calls[0][0].title).toBe("Too much to copy");
    });

    it("shows the bytes at the cursor as each type in the data inspector, in either byte order", async () => {
        await mount();
        expect(screen.queryByLabelText("Data inspector")).toBeNull();
        key("i");
        const value = (label: string) => Array.from(document.querySelectorAll(".hex-inspector-values dt")).find((dt) => dt.textContent === label)!.nextElementSibling!.textContent;
        // The cursor is on byte 0: 00 01 02 03 …
        expect(value("uint16")).toBe("256");
        expect(value("uint32")).toBe("50462976");
        fireEvent.click(screen.getByRole("radio", { name: "Big endian" }));
        expect(value("uint16")).toBe("1");
        expect(value("uint32")).toBe("66051");
        key("ArrowRight");
        expect(value("uint8")).toBe("1");
        fireEvent.click(screen.getByRole("button", { name: "Data inspector (I)" }));
        expect(screen.queryByLabelText("Data inspector")).toBeNull();
    });

    it("says a read failed, keeps the rows, and reads again on Try again", async () => {
        fetchMediaRange.mockRejectedValueOnce(new Error("HTTP 403"));
        await mount();
        expect(screen.getByRole("alert").textContent).toContain("Couldn't read the file: HTTP 403");
        expect(cell(0).textContent).toBe("··");
        fireEvent.click(screen.getByRole("button", { name: "Try again" }));
        await settle();
        expect(screen.queryByRole("alert")).toBeNull();
        expect(cell(1).textContent).toBe("01");
    });

    it("reads the file again when it changed on disk, and keeps the cursor inside a shorter file", async () => {
        const [revision, setRevision] = createSignal(0);
        const [size, setSize] = createSignal(FILE.length);
        render(() => (
            <HexViewer path="C:/bin/a.bin" revision={revision()} size={size()} memoryKey="C:/bin/a.bin" bar={{ viewer: "hex", options: ["hex"], onReopenWith: () => {} }} />
        ));
        await settle();
        key("ArrowDown");
        key("ArrowDown");
        key("ArrowDown");
        key("ArrowDown");
        key("ArrowDown");
        expect(cursorAt()).toBe(80);
        serve(Uint8Array.from({ length: 64 }, () => 0xee));
        setSize(64);
        setRevision(1);
        await settle();
        expect(cell(1).textContent).toBe("EE");
        expect(rowTexts()).toHaveLength(4);
        expect(cursorAt()).toBe(63);
    });

    it("comes back to a tab where it was: the row width, the cursor and the rows on screen", async () => {
        const first = await mount();
        fireEvent.click(screen.getByRole("radio", { name: "32" }));
        key("End", { ctrlKey: true });
        await settle();
        const top = rowTexts()[0];
        first.unmount();
        await mount();
        expect(rowTexts()[0]).toBe(top);
        expect(cursorAt()).toBe(FILE.length - 1);
        expect((screen.getByRole("radio", { name: "32" }) as HTMLElement).getAttribute("aria-checked")).toBe("true");
        // Another tab starts at the top.
        cleanup();
        await mount({ memoryKey: "C:/bin/other.bin" });
        expect(rowTexts()[0]).toBe("00000000");
    });

    it("puts the scroller back where the rows are when its pane is shown again", async () => {
        // A pane that was hidden (maximized past, a tab switched away) comes back with scrollTop 0 and no scroll event.
        const sizes: Array<() => void> = [];
        vi.stubGlobal("ResizeObserver", class { constructor(cb: () => void) { sizes.push(cb); } observe() {} unobserve() {} disconnect() {} });
        try {
            await mount();
            key("g", { ctrlKey: true });
            const box = screen.getByLabelText("Go to offset") as HTMLInputElement;
            fireEvent.input(box, { target: { value: "0x2027" } });
            fireEvent.keyDown(box, { key: "Enter" });
            await settle();
            const where = grid().scrollTop;
            const rows = rowTexts();
            expect(where).toBeGreaterThan(0);
            grid().scrollTop = 0;
            sizes.forEach((cb) => cb());
            await settle();
            expect(grid().scrollTop).toBe(where);
            expect(rowTexts()).toEqual(rows);
        } finally {
            vi.unstubAllGlobals();
        }
    });

    it("goes to an offset from a toolbar button's focus too", async () => {
        await mount();
        const button = screen.getByRole("radio", { name: "32" }) as HTMLElement;
        button.focus();
        fireEvent.keyDown(button, { key: "g", ctrlKey: true });
        expect(document.activeElement).toBe(screen.getByLabelText("Go to offset"));
    });
});
