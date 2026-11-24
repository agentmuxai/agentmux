// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Table viewer on screen: a CSV file as rows of cells, the header row
 * and its toggle, a file that can't be read, and an empty one.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4 (phase 5).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ROW_H } from "../hex/hex-layout";
import { TableViewer } from "./table-viewer";

const readRange = vi.hoisted(() => vi.fn());

vi.mock("../hex/hex-chunks", () => ({ rangeReaderOf: () => readRange }));

const VIEW_ROWS = 10;
const enc = (s: string) => new TextEncoder().encode(s);

/** A CSV of `rows` data rows under a header. */
const csv = (rows: number) => `id,name,note\n${Array.from({ length: rows }, (_, i) => `${i + 1},name ${i + 1},"note, ${i + 1}"`).join("\n")}\n`;

function serve(text: string) {
    const bytes = enc(text);
    readRange.mockImplementation(async (start: number, end: number) => bytes.slice(start, end + 1));
    return bytes.length;
}

const probeOf = (size: number, mime = "text/csv") => ({ size, mtime: 0, kind: "text", mime, read_only: false, text: { encoding: "utf-8", bom: "none" } });

async function settle() {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
}

async function mount(text: string, extra: { revision?: number; mime?: string } = {}) {
    const size = serve(text);
    const view = render(() => (
        <TableViewer
            path="C:/data/people.csv"
            revision={extra.revision ?? 0}
            probe={probeOf(size, extra.mime)}
            bar={{ viewer: "table", options: ["text", "table", "info", "hex"], onReopenWith: () => {} }}
        />
    ));
    await settle();
    return view;
}

const cells = (row: Element) => Array.from(row.querySelectorAll(".table-viewer-cell")).map((c) => c.textContent);
const bodyRows = () => Array.from(document.querySelectorAll(".table-viewer-rows .table-viewer-row"));
const headRow = () => document.querySelector(".table-viewer-head");
const facts = () => Array.from(document.querySelectorAll(".editor-viewer-bar-facts span")).map((s) => s.textContent);

describe("the Table viewer", () => {
    let clientHeight: PropertyDescriptor | undefined;

    beforeEach(() => {
        readRange.mockReset();
        clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
        Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => ROW_H * VIEW_ROWS });
    });

    afterEach(() => {
        cleanup();
        if (clientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
    });

    it("shows the first row as the header and draws only the rows on screen, numbered from 1", async () => {
        await mount(csv(500));
        expect(cells(headRow()!)).toEqual(["id", "name", "note"]);
        expect(bodyRows()).toHaveLength(VIEW_ROWS + 2);
        expect(cells(bodyRows()[0])).toEqual(["1", "name 1", "note, 1"]);
        expect(bodyRows()[0].querySelector(".table-viewer-number")?.textContent).toBe("1");
        expect(facts().slice(0, 3)).toEqual(["500 rows", "3 columns", "comma"]);
    });

    it("turns the header off and on: the first row joins the rows, and the count follows", async () => {
        await mount(csv(20));
        const toggle = screen.getByRole("button", { name: "Header row" });
        expect(toggle.getAttribute("aria-pressed")).toBe("true");
        fireEvent.click(toggle);
        await settle();
        expect(toggle.getAttribute("aria-pressed")).toBe("false");
        expect(headRow()).toBeNull();
        expect(cells(bodyRows()[0])).toEqual(["id", "name", "note"]);
        expect(facts()[0]).toBe("21 rows");
        fireEvent.click(toggle);
        await settle();
        expect(headRow()).not.toBeNull();
        expect(facts()[0]).toBe("20 rows");
    });

    it("reads a TSV by the file's type, and sniffs a delimiter when the type doesn't say", async () => {
        await mount("a\tb\n1\t2\n", { mime: "text/tab-separated-values" });
        expect(cells(headRow()!)).toEqual(["a", "b"]);
        expect(facts()[2]).toBe("tab");
        cleanup();
        await mount("a;b;c\n1;2;3\n");
        expect(cells(headRow()!)).toEqual(["a", "b", "c"]);
        expect(facts()[2]).toBe("semicolon");
    });

    it("keeps a quoted cell's delimiter, quotes and line breaks in the cell", async () => {
        await mount('h1,h2\n"a, b","say ""hi"""\n');
        expect(cells(bodyRows()[0])).toEqual(["a, b", 'say "hi"']);
    });

    it("reads the file again when it changed on disk", async () => {
        const [revision, setRevision] = createSignal(0);
        const [size, setSize] = createSignal(serve("a,b\n1,2\n"));
        render(() => (
            <TableViewer
                path="C:/data/people.csv"
                revision={revision()}
                probe={probeOf(size())}
                bar={{ viewer: "table", options: ["text", "table", "info", "hex"], onReopenWith: () => {} }}
            />
        ));
        await settle();
        expect(facts()[0]).toBe("1 row");
        // The probe is read again before the revision moves, as the editor does.
        setSize(serve("a,b\n1,2\n3,4\n5,6\n"));
        setRevision(1);
        await settle();
        expect(facts()[0]).toBe("3 rows");
    });

    it("says an empty file is empty, and says when the file can't be read", async () => {
        await mount("");
        expect(document.querySelector(".editor-viewer-message")?.textContent).toBe("This file is empty.");
        cleanup();
        readRange.mockReset();
        readRange.mockRejectedValue(new Error("gone"));
        render(() => <TableViewer path="C:/data/x.csv" revision={0} probe={probeOf(10)} bar={{ viewer: "table", options: ["text", "table"], onReopenWith: () => {} }} />);
        await settle();
        expect(screen.getByRole("alert").textContent).toBe("Couldn't read the file: gone");
    });
});
