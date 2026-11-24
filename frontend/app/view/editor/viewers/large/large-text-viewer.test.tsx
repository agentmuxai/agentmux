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
import { clearOpenPositions, setOpenPosition, takeOpenPosition } from "../../open-position";
import { ROW_H } from "../hex/hex-layout";
import { LargeTextViewer } from "./large-text-viewer";
import { forgetLargeTextMemory, parseLine, revealSideways } from "./large-text-state";
import { LINE_SHOWN_MAX, PAGE_LINES } from "./line-source";

const fetchMediaRange = vi.hoisted(() => vi.fn());
const indexLines = vi.hoisted(() => vi.fn());

vi.mock("@/app/element/local-media", async (actual) => ({ ...(await actual<object>()), fetchMediaRange }));
vi.mock("@/app/store/global", () => ({ pushNotification: vi.fn() }));
const searchBytes = vi.hoisted(() => vi.fn());
const searchText = vi.hoisted(() => vi.fn());

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { FileIndexLinesCommand: indexLines, FileSearchBytesCommand: searchBytes, FileSearchTextCommand: searchText },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

const VIEW_LINES = 10;

/** Serve `text` as srv's line index and a range read would. */
function serve(text: string) {
    const bytes = new TextEncoder().encode(text);
    const starts = text.length ? [0] : [];
    for (let i = 0; i < bytes.length; i++) if (bytes[i] === 10 && i + 1 < bytes.length) starts.push(i + 1);
    indexLines.mockImplementation(async (_c: unknown, req: { from_line: number; count: number; at_offset?: number; encoding?: string }) => {
        const at = (n: number) => (n >= starts.length ? bytes.length : starts[n]);
        const line_at = req.at_offset == null ? undefined : starts.filter((s) => s <= req.at_offset!).length - 1;
        const from = line_at ?? req.from_line;
        return { total_lines: starts.length, size: bytes.length, mtime: 1, start: at(from), end: at(from + req.count), encoding: req.encoding === "legacy" ? "windows-1252" : (req.encoding ?? "utf-8"), starts: [], line_at };
    });
    // srv's byte search: the first match at or after `from`, the last at or before it going backward.
    searchBytes.mockImplementation(async (_c: unknown, req: { pattern: string; from: number; backward: boolean }) => {
        const needle = String.fromCharCode(...req.pattern.split(" ").map((h) => parseInt(h, 16)));
        const hay = String.fromCharCode(...bytes);
        const offset = req.backward ? hay.lastIndexOf(needle, req.from) : hay.indexOf(needle, req.from);
        return { found: offset >= 0, offset: Math.max(0, offset), length: needle.length, done: offset < 0, next_from: 0 };
    });
    // srv's text search: a literal or a regular expression, either case, as filesearchtext does (forward only here).
    searchText.mockImplementation(async (_c: unknown, req: { query: string; regex: boolean; ignore_case: boolean; from: number; backward: boolean }) => {
        const hay = String.fromCharCode(...bytes);
        const re = new RegExp(req.regex ? req.query : req.query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), req.ignore_case ? "gim" : "gm");
        re.lastIndex = req.from;
        const m = re.exec(hay);
        return { found: !!m, offset: m?.index ?? 0, length: m?.[0].length ?? 0, done: !m, next_from: 0 };
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

async function mount(props: { path?: string; revision?: number; size?: number; encoding?: string; blockId?: string } = {}) {
    const view = render(() => (
        <LargeTextViewer
            path={props.path ?? "C:/logs/big.log"}
            revision={props.revision ?? 0}
            blockId={props.blockId}
            probe={{ size: props.size ?? FILE.length, mtime: 0, kind: "text", mime: "text/plain", read_only: false, text: { encoding: props.encoding ?? "utf-8", bom: "none" } }}
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
        searchBytes.mockReset();
        searchText.mockReset();
        serve(FILE);
        clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
        Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => ROW_H * VIEW_LINES });
    });

    afterEach(() => {
        cleanup();
        clearOpenPositions();
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

    it("opens at the line pane.open asked for, once the line count is known, and only once", async () => {
        setOpenPosition("b1", "C:/logs/big.log", { line: 700 });
        await mount({ blockId: "b1" });
        await settle();
        expect(document.querySelector(".large-text-marked")?.getAttribute("data-line")).toBe("700");
        expect(facts()[0]).toBe("Line 698 of 1,000");
        // Taken: a later mount of the same file starts at the top again.
        cleanup();
        forgetLargeTextMemory();
        await mount({ blockId: "b1" });
        expect(facts()[0]).toBe("Line 1 of 1,000");
    });

    it("clamps a line past the end to the last line, and takes none meant for another file", async () => {
        setOpenPosition("b1", "C:/logs/other.log", { line: 5 });
        setOpenPosition("b1", "C:/logs/big.log", { line: 99999 });
        await mount({ blockId: "b1" });
        await settle();
        expect(document.querySelector(".large-text-marked")?.getAttribute("data-line")).toBe("1000");
        expect(takeOpenPosition("b1", "C:/logs/other.log", "line")).toBe(5);
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

    it("moves a file of more than 400,000 lines by the distance the wheel moved, small events and all", async () => {
        indexLines.mockImplementation(async () => ({ total_lines: 500_000, size: 10_000_000, mtime: 1, start: 0, end: 0, encoding: "utf-8", starts: [] }));
        await mount();
        // A trackpad: sixty events of a quarter line each are fifteen lines, not sixty.
        for (let i = 0; i < 60; i++) fireEvent.wheel(grid(), { deltaY: ROW_H / 4, deltaMode: 0 });
        expect(numbers()[0]).toBe(16);
        // One notch of a wheel: six lines.
        fireEvent.wheel(grid(), { deltaY: ROW_H * 6, deltaMode: 0 });
        expect(numbers()[0]).toBe(22);
        fireEvent.wheel(grid(), { deltaY: -ROW_H * 2, deltaMode: 0 });
        expect(numbers()[0]).toBe(20);
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

    const hex = (s: string) => Array.from(s, (c) => c.charCodeAt(0).toString(16).toUpperCase().padStart(2, "0")).join(" ");
    const findBox = () => screen.getByLabelText("Find in the file", { selector: "input" }) as HTMLInputElement;
    const markedLine = () => document.querySelector(".large-text-marked")?.getAttribute("data-line");

    it("finds text on srv from the top of the screen, shows its line with the text marked, and goes on from the match", async () => {
        await mount();
        key("f", { ctrlKey: true });
        await settle();
        expect(document.activeElement).toBe(findBox());
        fireEvent.input(findBox(), { target: { value: "line 5" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        // From the first byte on screen; "line 5" is the fifth line, 28 bytes in.
        expect(searchBytes).toHaveBeenLastCalledWith({}, { path: "C:/logs/big.log", pattern: hex("line 5"), from: 0, backward: false });
        expect(markedLine()).toBe("5");
        expect(document.querySelector(".large-text-hit")?.textContent).toBe("line 5");
        expect(document.querySelector(".large-text-find-status")?.textContent).toBe("Line 5");
        // The next match starts past this one; the previous one, before it.
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(searchBytes).toHaveBeenLastCalledWith({}, { path: "C:/logs/big.log", pattern: hex("line 5"), from: 29, backward: false });
        expect(markedLine()).toBe("50");
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(markedLine()).toBe("51");
        fireEvent.keyDown(findBox(), { key: "Enter", shiftKey: true });
        await settle();
        expect(markedLine()).toBe("50");
    });

    it("ignores case when Match case is off: srv is asked for text, and the match is marked as it is written in the line", async () => {
        await mount();
        key("f", { ctrlKey: true });
        await settle();
        const matchCase = screen.getByRole("button", { name: "Match case" });
        expect(matchCase.getAttribute("aria-pressed")).toBe("true");
        fireEvent.click(matchCase);
        expect(matchCase.getAttribute("aria-pressed")).toBe("false");
        fireEvent.input(findBox(), { target: { value: "LINE 5" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(searchBytes).not.toHaveBeenCalled();
        expect(searchText).toHaveBeenLastCalledWith({}, { path: "C:/logs/big.log", query: "LINE 5", regex: false, ignore_case: true, from: 0, backward: false });
        expect(markedLine()).toBe("5");
        expect(document.querySelector(".large-text-hit")?.textContent).toBe("line 5");
    });

    it("asks again at once when Match case or Regular expression is toggled with a text typed, and not with none", async () => {
        await mount();
        key("f", { ctrlKey: true });
        await settle();
        fireEvent.click(screen.getByRole("button", { name: "Match case" }));
        await settle();
        expect(searchText).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Match case" }));
        fireEvent.input(findBox(), { target: { value: "LINE 5" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(document.querySelector(".large-text-find-status")?.textContent).toBe("No match");
        searchText.mockClear();
        fireEvent.click(screen.getByRole("button", { name: "Match case" }));
        await settle();
        expect(searchText).toHaveBeenCalledTimes(1);
        expect(searchText).toHaveBeenLastCalledWith({}, { path: "C:/logs/big.log", query: "LINE 5", regex: false, ignore_case: true, from: 0, backward: false });
        expect(markedLine()).toBe("5");
        searchText.mockClear();
        fireEvent.click(screen.getByRole("button", { name: "Regular expression" }));
        await settle();
        expect(searchText).toHaveBeenCalledTimes(1);
        expect(searchText).toHaveBeenLastCalledWith({}, { path: "C:/logs/big.log", query: "LINE 5", regex: true, ignore_case: true, from: 0, backward: false });
    });

    it("finds a regular expression on srv and marks what it matched", async () => {
        await mount();
        key("f", { ctrlKey: true });
        await settle();
        fireEvent.click(screen.getByRole("button", { name: "Regular expression" }));
        fireEvent.input(findBox(), { target: { value: "line 9\\d$" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(searchText).toHaveBeenLastCalledWith({}, { path: "C:/logs/big.log", query: "line 9\\d$", regex: true, ignore_case: false, from: 0, backward: false });
        expect(markedLine()).toBe("90");
        expect(document.querySelector(".large-text-hit")?.textContent).toBe("line 90");
    });

    it("shows the reason srv gave for a bad expression, without its command name", async () => {
        await mount();
        key("f", { ctrlKey: true });
        await settle();
        fireEvent.click(screen.getByRole("button", { name: "Regular expression" }));
        searchText.mockRejectedValueOnce(new Error("filesearchtext: not a valid expression: unclosed group"));
        fireEvent.input(findBox(), { target: { value: "Err(" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(document.querySelector(".large-text-find-status")?.textContent).toBe("Couldn't search: not a valid expression: unclosed group");
    });

    it("goes on past the whole match of an expression, never to the tail of the match on show", async () => {
        await mount();
        key("f", { ctrlKey: true });
        await settle();
        fireEvent.click(screen.getByRole("button", { name: "Regular expression" }));
        fireEvent.input(findBox(), { target: { value: "\\d+" } });
        for (let line = 1; line <= 10; line++) {
            fireEvent.keyDown(findBox(), { key: "Enter" });
            await settle();
            expect(markedLine()).toBe(String(line));
        }
        // On "line 10" the match is "10": the next one is on line 11, not "0" of line 10.
        expect(document.querySelector(".large-text-hit-current")?.textContent).toBe("10");
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(markedLine()).toBe("11");
    });

    it("says a UTF-16 file can't be searched by expression, without asking srv; turning the option off searches again", async () => {
        await mount({ encoding: "utf-16le" });
        key("f", { ctrlKey: true });
        await settle();
        fireEvent.click(screen.getByRole("button", { name: "Regular expression" }));
        fireEvent.input(findBox(), { target: { value: "a+" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(document.querySelector(".large-text-find-status")?.textContent).toBe("A regular expression can only search a UTF-8 file; this one is in utf-16le");
        expect(searchText).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Regular expression" }));
        expect(document.querySelector(".large-text-find-status")?.textContent ?? "").not.toContain("UTF-8");
    });

    it("finds a match far from the screen and scrolls to it", async () => {
        await mount();
        key("f", { ctrlKey: true });
        await settle();
        fireEvent.input(findBox(), { target: { value: "line 900" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(markedLine()).toBe("900");
        expect(numbers()).toContain(900);
        expect(document.querySelector('[data-line="900"] .large-text-hit')?.textContent).toBe("line 900");
    });

    it("scrolls sideways to a match on a line it has read before, not only the first time the line is visited", async () => {
        // jsdom has no layout: the match sits 900 px along a grid that is 371 px wide.
        const was = HTMLElement.prototype.getBoundingClientRect;
        HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
            const [left, right] = this.classList.contains("large-text-hit") ? [900, 960] : [0, 371];
            return { left, right, width: right - left, x: left, top: 0, bottom: 0, height: 0, y: 0, toJSON: () => ({}) } as DOMRect;
        };
        try {
            await mount();
            key("f", { ctrlKey: true });
            await settle();
            fireEvent.input(findBox(), { target: { value: "line 5" } });
            const step = async (init: KeyboardEventInit, line: string) => {
                grid().scrollLeft = 0;
                fireEvent.keyDown(findBox(), { key: "Enter", ...init });
                await settle();
                await new Promise((r) => setTimeout(r, 80));
                expect(markedLine()).toBe(line);
                expect(grid().scrollLeft).toBeGreaterThan(0);
            };
            await step({}, "5"); // the first visit: the line is read now
            await step({}, "50");
            await step({ shiftKey: true }, "5"); // a visit to a line already read
        } finally {
            HTMLElement.prototype.getBoundingClientRect = was;
        }
    });

    it("goes through the matches of one line one at a time: the current one is marked and scrolled to, not the first again", async () => {
        const text = "é err y err z err w\nline 2\nline 3\n";
        serve(text);
        // jsdom has no layout: the hits of a line sit 300 px apart along a grid that is 371 px wide.
        const was = HTMLElement.prototype.getBoundingClientRect;
        HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
            const hits = Array.from(this.parentElement?.querySelectorAll(".large-text-hit") ?? []);
            const [left, right] = this.classList.contains("large-text-hit") ? [300 * (hits.indexOf(this) + 1), 300 * (hits.indexOf(this) + 1) + 60] : [0, 371];
            return { left, right, width: right - left, x: left, top: 0, bottom: 0, height: 0, y: 0, toJSON: () => ({}) } as DOMRect;
        };
        try {
            await mount({ size: new TextEncoder().encode(text).length });
            key("f", { ctrlKey: true });
            await settle();
            fireEvent.input(findBox(), { target: { value: "err" } });
            const hitsInLine = () => Array.from(document.querySelectorAll('[data-line="1"] .large-text-hit'));
            const currentHit = () => hitsInLine().findIndex((h) => h.classList.contains("large-text-hit-current"));
            const step = async (init: KeyboardEventInit) => {
                grid().scrollLeft = 0;
                fireEvent.keyDown(findBox(), { key: "Enter", ...init });
                await settle();
                await new Promise((r) => setTimeout(r, 80));
                expect(markedLine()).toBe("1");
                expect(hitsInLine()).toHaveLength(3);
                expect(document.querySelectorAll(".large-text-hit-current")).toHaveLength(1);
            };
            await step({});
            expect(currentHit()).toBe(0);
            expect(grid().scrollLeft).toBe(0); // the first is on screen already
            await step({});
            expect(currentHit()).toBe(1);
            expect(grid().scrollLeft).toBeGreaterThan(0); // the second is off screen: the same line, and it is revealed
            await step({});
            expect(currentHit()).toBe(2);
            expect(grid().scrollLeft).toBeGreaterThan(0);
            await step({ shiftKey: true });
            expect(currentHit()).toBe(1);
        } finally {
            HTMLElement.prototype.getBoundingClientRect = was;
        }
    });

    it("says there is no match, and refuses text it cannot search in the file's encoding without asking srv", async () => {
        await mount();
        key("f", { ctrlKey: true });
        await settle();
        fireEvent.input(findBox(), { target: { value: "nowhere" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(document.querySelector(".large-text-find-status")?.textContent).toBe("No match");
        expect(markedLine()).toBeUndefined();
        cleanup();
        searchBytes.mockClear();
        await mount({ encoding: "legacy" });
        key("f", { ctrlKey: true });
        await settle();
        fireEvent.input(findBox(), { target: { value: "é" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(searchBytes).not.toHaveBeenCalled();
        // srv's own name for the file's encoding is what the message gives, not the probe's "legacy".
        expect(document.querySelector(".large-text-find-error")?.textContent).toContain("windows-1252");
    });

    it("refuses any search in a multibyte legacy encoding, saying why, without asking srv", async () => {
        await mount({ encoding: "Shift_JIS" });
        key("f", { ctrlKey: true });
        await settle();
        fireEvent.input(findBox(), { target: { value: "a" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(searchBytes).not.toHaveBeenCalled();
        expect(document.querySelector(".large-text-find-error")?.textContent).toContain("several bytes");
    });

    it("takes the mark off the line when a later search finds nothing, so the old line is not taken for its answer", async () => {
        await mount();
        key("f", { ctrlKey: true });
        await settle();
        fireEvent.input(findBox(), { target: { value: "line 5" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(markedLine()).toBe("5");
        fireEvent.input(findBox(), { target: { value: "nowhere" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        expect(markedLine()).toBeUndefined();
        expect(document.querySelector(".large-text-hit")).toBeNull();
    });

    it("opens from the line box too, and closes on Escape with the focus back on the lines", async () => {
        await mount();
        fireEvent.keyDown(lineBox(), { key: "f", ctrlKey: true });
        await settle();
        expect(document.activeElement).toBe(findBox());
        fireEvent.keyDown(findBox(), { key: "Escape" });
        expect(screen.queryByRole("search")).toBeNull();
        expect(document.activeElement).toBe(grid());
    });

    it("drops a search that was running when the file changed", async () => {
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
        key("f", { ctrlKey: true });
        await settle();
        let answer: (r: unknown) => void = () => {};
        searchBytes.mockReturnValue(new Promise((resolve) => (answer = resolve)));
        fireEvent.input(findBox(), { target: { value: "line 5" } });
        fireEvent.keyDown(findBox(), { key: "Enter" });
        await settle();
        setRevision(1);
        await settle();
        answer({ found: true, offset: 28, length: 6, done: false, next_from: 0 });
        await settle();
        expect(markedLine()).toBeUndefined();
        expect(document.querySelector(".large-text-find-status")?.textContent ?? "").not.toContain("Line");
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

describe("revealSideways", () => {
    const rect = (left: number, right: number) => ({ left, right, width: right - left }) as DOMRect;
    const grid = (left: number, right: number) => ({ getBoundingClientRect: () => rect(left, right), scrollLeft: 0 }) as unknown as HTMLElement;
    const hit = (left: number, right: number) => ({ getBoundingClientRect: () => rect(left, right) }) as unknown as Element;

    it("scrolls a match that is off to the right into the middle of the view", () => {
        const g = grid(0, 371);
        revealSideways(g, hit(925, 985));
        // 925 - (371 - 60) / 2: the match's left edge ends up where it centres.
        expect(g.scrollLeft).toBeCloseTo(925 - 155.5);
    });

    it("scrolls back for a match that is off to the left, and leaves one that shows alone", () => {
        const g = grid(100, 471);
        g.scrollLeft = 800;
        revealSideways(g, hit(-300, -240));
        expect(g.scrollLeft).toBeLessThan(800);
        const still = grid(0, 371);
        revealSideways(still, hit(40, 100));
        expect(still.scrollLeft).toBe(0);
        revealSideways(still, null);
        revealSideways(undefined, hit(0, 1));
    });
});
