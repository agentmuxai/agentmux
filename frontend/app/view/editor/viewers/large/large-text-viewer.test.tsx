// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Large text viewer on screen: the lines it draws from a file read a page
 * at a time, go to a line, the keys that scroll, long lines, an empty file, a
 * read that fails, and coming back to a tab.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ROW_H } from "../hex/hex-layout";
import { LargeTextViewer } from "./large-text-viewer";
import { forgetLargeTextMemory, parseLine } from "./large-text-state";
import { LINE_SHOWN_MAX, PAGE_LINES } from "./line-source";

const fetchMediaRange = vi.hoisted(() => vi.fn());
const indexLines = vi.hoisted(() => vi.fn());

vi.mock("@/app/element/local-media", async (actual) => ({ ...(await actual<object>()), fetchMediaRange }));
vi.mock("@/app/store/global", () => ({ pushNotification: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { FileIndexLinesCommand: indexLines } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

const VIEW_LINES = 10;

/** Serve `text` as srv's line index and a range read would. */
function serve(text: string) {
    const bytes = new TextEncoder().encode(text);
    const starts = text.length ? [0] : [];
    for (let i = 0; i < bytes.length; i++) if (bytes[i] === 10 && i + 1 < bytes.length) starts.push(i + 1);
    indexLines.mockImplementation(async (_c: unknown, req: { from_line: number; count: number }) => {
        const at = (n: number) => (n >= starts.length ? bytes.length : starts[n]);
        return { total_lines: starts.length, size: bytes.length, mtime: 1, start: at(req.from_line), end: at(req.from_line + req.count) };
    });
    fetchMediaRange.mockImplementation(async (_path: string, start: number, end: number) => ({
        blob: { arrayBuffer: async () => bytes.slice(start, end + 1).buffer } as unknown as Blob,
        total: bytes.length,
    }));
}

const lineText = (i: number) => `line ${i}`;
const FILE = Array.from({ length: 1000 }, (_, i) => lineText(i + 1)).join("\n") + "\n";

async function settle() {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
}

async function mount(props: { path?: string; revision?: number; size?: number } = {}) {
    const view = render(() => (
        <LargeTextViewer
            path={props.path ?? "C:/logs/big.log"}
            revision={props.revision ?? 0}
            probe={{ size: props.size ?? FILE.length, mtime: 0, kind: "text", mime: "text/plain", read_only: false, text: { encoding: "utf-8", bom: "none" } }}
            bar={{ viewer: "large", options: ["large", "info", "hex"], onReopenWith: () => {} }}
        />
    ));
    await settle();
    return view;
}

const grid = () => screen.getByRole("region", { name: "The file's lines" });
const numbers = () => Array.from(document.querySelectorAll(".large-text-line")).map((l) => Number(l.getAttribute("data-line")));
const texts = () => Array.from(document.querySelectorAll(".large-text-text")).map((l) => l.textContent);
const facts = () => Array.from(document.querySelectorAll(".editor-viewer-bar-facts span")).map((s) => s.textContent);
const lineBox = () => screen.getByLabelText("Go to line") as HTMLInputElement;
const key = (k: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(grid(), { key: k, ...init });

describe("the Large text viewer", () => {
    let clientHeight: PropertyDescriptor | undefined;

    beforeEach(() => {
        forgetLargeTextMemory();
        fetchMediaRange.mockReset();
        indexLines.mockReset();
        serve(FILE);
        clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
        Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => ROW_H * VIEW_LINES });
    });

    afterEach(() => {
        cleanup();
        if (clientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
    });

    it("draws only the lines on screen, numbered, from one page of the file", async () => {
        await mount();
        expect(numbers()).toEqual(Array.from({ length: VIEW_LINES + 2 }, (_, i) => i + 1));
        expect(texts().slice(0, 2)).toEqual(["line 1", "line 2"]);
        expect(indexLines).toHaveBeenCalledTimes(1);
        expect(indexLines.mock.calls[0][1]).toMatchObject({ path: "C:/logs/big.log", encoding: "utf-8", from_line: 0, count: PAGE_LINES });
        // The cold index of a big file reads all of it: more than the default time.
        expect(indexLines.mock.calls[0][2].timeout).toBeGreaterThan(5000);
        expect(facts()).toEqual(["Line 1 of 1,000", "8.7 KB"]);
    });

    it("goes to a line, and marks it", async () => {
        await mount();
        fireEvent.input(lineBox(), { target: { value: "700" } });
        fireEvent.keyDown(lineBox(), { key: "Enter" });
        await settle();
        expect(numbers()).toContain(700);
        expect(document.querySelector(".large-text-marked")?.getAttribute("data-line")).toBe("700");
        expect(document.querySelector('[data-line="700"] .large-text-text')?.textContent).toBe("line 700");
        expect(facts()[0]).toBe("Line 698 of 1,000");
        // Only the pages it needed: the first, and the one holding line 700 (a little context above it).
        expect(indexLines.mock.calls.map((c) => c[1].from_line).sort((a, b) => a - b)).toEqual([0, 600]);
    });

    it("says so when the line isn't one", async () => {
        await mount();
        fireEvent.input(lineBox(), { target: { value: "1001" } });
        fireEvent.keyDown(lineBox(), { key: "Enter" });
        expect(lineBox().getAttribute("aria-invalid")).toBe("true");
        fireEvent.input(lineBox(), { target: { value: "5" } });
        expect(lineBox().getAttribute("aria-invalid")).toBe("false");
    });

    it("scrolls by a line, a page, to the top and to the end with the keys", async () => {
        await mount();
        key("ArrowDown");
        expect(numbers()[0]).toBe(2);
        key("PageDown");
        expect(numbers()[0]).toBe(2 + (VIEW_LINES - 1));
        key("PageUp");
        expect(numbers()[0]).toBe(2);
        key("End", { ctrlKey: true });
        await settle();
        expect(numbers().at(-1)).toBe(1000);
        expect(texts().at(-1)).toBe("line 1000");
        key("Home", { ctrlKey: true });
        expect(numbers()[0]).toBe(1);
    });

    it("leaves a modified key to the app", async () => {
        await mount();
        const e = new KeyboardEvent("keydown", { key: "w", ctrlKey: true, bubbles: true, cancelable: true });
        grid().dispatchEvent(e);
        expect(e.defaultPrevented).toBe(false);
    });

    it("cuts a long line and says how much is hidden", async () => {
        serve(`short\n${"x".repeat(LINE_SHOWN_MAX + 500)}\nafter\n`);
        await mount();
        expect(texts()[0]).toBe("short");
        expect(document.querySelector('[data-line="2"] .large-text-hidden')?.textContent).toBe("…(+500 characters)");
        expect(document.querySelector('[data-line="3"] .large-text-text')?.textContent).toBe("after");
    });

    it("says an empty file is empty", async () => {
        serve("");
        await mount({ size: 0 });
        expect(screen.getByText("This file is empty.")).toBeTruthy();
    });

    it("says when the file can't be read, and tries again on request", async () => {
        indexLines.mockRejectedValueOnce(new Error("EC-TIME: timeout"));
        await mount();
        expect(screen.getByRole("alert").textContent).toContain("Couldn't read the file: EC-TIME: timeout");
        fireEvent.click(screen.getByText("Try again"));
        await settle();
        expect(screen.queryByRole("alert")).toBeNull();
        expect(texts()[0]).toBe("line 1");
    });

    it("comes back to the line a tab was at", async () => {
        const first = await mount();
        fireEvent.input(lineBox(), { target: { value: "500" } });
        fireEvent.keyDown(lineBox(), { key: "Enter" });
        await settle();
        const was = numbers()[0];
        first.unmount();
        await mount();
        expect(numbers()[0]).toBe(was);
    });

    it("reads the file again when it changed on disk, and stays at the line it was at", async () => {
        const [revision, setRevision] = createSignal(0);
        render(() => (
            <LargeTextViewer
                path="C:/logs/big.log"
                revision={revision()}
                probe={{ size: FILE.length, mtime: 0, kind: "text", mime: "text/plain", read_only: false, text: { encoding: "utf-8", bom: "none" } }}
                bar={{ viewer: "large", options: ["large", "info", "hex"], onReopenWith: () => {} }}
            />
        ));
        await settle();
        fireEvent.input(lineBox(), { target: { value: "500" } });
        fireEvent.keyDown(lineBox(), { key: "Enter" });
        await settle();
        const was = numbers()[0];
        expect(was).toBeGreaterThan(400);
        indexLines.mockClear();
        // The log grew by a line.
        serve(`${FILE}line 1001\n`);
        setRevision(1);
        await settle();
        expect(indexLines).toHaveBeenCalled();
        expect(numbers()[0]).toBe(was);
        expect(document.querySelector(`[data-line="${was}"] .large-text-text`)?.textContent).toBe(`line ${was}`);
        expect(facts()[0]).toBe(`Line ${was} of 1,001`);
    });
});

describe("parseLine", () => {
    it("takes a line number inside the file, with separators", () => {
        expect(parseLine("1", 10)).toBe(1);
        expect(parseLine(" 1,000 ", 5000)).toBe(1000);
        expect(parseLine("10", 10)).toBe(10);
        for (const bad of ["", "0", "11", "-3", "1.5", "0x10", "abc"]) expect(parseLine(bad, 10), bad).toBeNull();
    });
});
