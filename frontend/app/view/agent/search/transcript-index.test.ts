// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { DocumentNode } from "../../../agent-model/types";
import {
    createTranscriptIndex,
    firstMatchFrom,
    FOLD_CAP,
    searchableText,
    topRowId,
    type PassClock,
} from "./transcript-index";

const md = (id: string, content: string): DocumentNode => ({ type: "markdown", id, content }) as DocumentNode;
const never = () => false;

/** A clock that reads the number of rows folded so far: each row costs one unit of time. */
function costClock(folds: () => number, yields: number[]): PassClock {
    return {
        now: () => folds(),
        yieldNow: () => {
            yields.push(folds());
            return Promise.resolve();
        },
    };
}

const immediate: PassClock = { now: () => 0, yieldNow: () => Promise.resolve() };

describe("searchableText", () => {
    it("reads the text of each kind of row the search covers", () => {
        const text = (n: object) => searchableText(n as DocumentNode);
        expect(text({ type: "markdown", id: "a", content: "thought about X" })).toBe("thought about X");
        expect(text({ type: "user_message", id: "a", message: "hello" })).toBe("hello");
        expect(text({ type: "agent_message", id: "a", message: "from a peer" })).toBe("from a peer");
        expect(text({ type: "tool", id: "a", tool: "Read", params: { file_path: "/a/b.ts" } })).toBe(
            'Read {"file_path":"/a/b.ts"}'
        );
        expect(text({ type: "tool", id: "a", tool: "Read" })).toBe("Read {}");
        expect(text({ type: "shell", id: "a", cmd: "ls -la", title: "List" })).toBe("ls -la List");
        expect(text({ type: "jekt_message", id: "a", from: "x", to: "y", message: "hi" })).toBe("x y hi");
        expect(text({ type: "system_note", id: "a", text: "compacted" })).toBe("compacted");
    });

    it("reads nothing from a row of any other kind", () => {
        expect(searchableText({ type: "day_divider", id: "d" } as unknown as DocumentNode)).toBe("");
        expect(
            searchableText({
                type: "hidden_run",
                id: "h",
                counts: [],
                failed: 0,
                open: false,
                hiddenIds: [],
            } as DocumentNode)
        ).toBe("");
    });
});

describe("createTranscriptIndex", () => {
    it("finds the rows holding the query, ignoring case, in document order", async () => {
        const index = createTranscriptIndex(immediate);
        const nodes = [md("a", "Hello World"), md("b", "nothing"), md("c", "say HELLO"), md("d", "hell")];
        expect(await index.search(nodes, "hello", never)).toEqual(["a", "c"]);
    });

    it("folds a row once: a second query reads the cache, a changed row is folded alone", async () => {
        const index = createTranscriptIndex(immediate);
        const nodes = [md("a", "one"), md("b", "two"), md("c", "three")];
        await index.search(nodes, "o", never);
        expect(index.folds()).toBe(3);
        await index.search(nodes, "t", never);
        expect(index.folds()).toBe(3);
        // A streamed row: the same id, a new object.
        const grown = [nodes[0], md("b", "two more"), nodes[2]];
        expect(await index.search(grown, "more", never)).toEqual(["b"]);
        expect(index.folds()).toBe(4);
    });

    it("drops rows that left the document, so a different slice does not keep the old one alive", async () => {
        const index = createTranscriptIndex(immediate);
        const first = [md("a", "x"), md("b", "x")];
        await index.search(first, "x", never);
        await index.search([first[1]], "x", never);
        await index.search(first, "x", never);
        // "a" was folded, forgotten with the slice that left it, and folded again.
        expect(index.folds()).toBe(3);
    });

    it("searches only the first 64 KB of a row", async () => {
        const index = createTranscriptIndex(immediate);
        const row = md("a", "x".repeat(FOLD_CAP) + "needle");
        expect(await index.search([row], "needle", never)).toEqual([]);
        expect(await index.search([md("b", "x".repeat(FOLD_CAP - 6) + "needle")], "needle", never)).toEqual(["b"]);
    });

    it("keeps ids and order stable when rows are added before a match", async () => {
        const index = createTranscriptIndex(immediate);
        const base = [md("b", "hit"), md("c", "hit")];
        expect(await index.search(base, "hit", never)).toEqual(["b", "c"]);
        expect(await index.search([md("a", "hit"), ...base], "hit", never)).toEqual(["a", "b", "c"]);
    });

    it("yields between slices, scans no row twice, and never runs a slice past its budget", async () => {
        let folds = () => 0;
        const yields: number[] = [];
        const index = createTranscriptIndex(
            {
                now: () => folds(),
                yieldNow: () => {
                    yields.push(folds());
                    return Promise.resolve();
                },
            },
            8
        );
        folds = index.folds;
        const body = "y".repeat(2048);
        const nodes = Array.from({ length: 20_000 }, (_, i) =>
            md(`n${i}`, `${body}${i % 1000 === 0 ? " needle" : ""}`)
        );
        const found = await index.search(nodes, "needle", never);
        expect(found).toHaveLength(20);
        expect(index.folds()).toBe(20_000);
        // Each slice is the clock's check interval (32 rows) at most: that is the algorithm's shape, not a timing.
        const gaps = yields.map((at, i) => at - (yields[i - 1] ?? 0));
        expect(Math.max(...gaps)).toBeLessThanOrEqual(32);
        expect(yields.length).toBe(Math.ceil(20_000 / 32) - 1);
    });

    it("stops and answers null when the query changes between slices", async () => {
        let folds = () => 0;
        const yields: number[] = [];
        const index = createTranscriptIndex(
            costClock(() => folds(), yields),
            8
        );
        folds = index.folds;
        const nodes = Array.from({ length: 500 }, (_, i) => md(`n${i}`, "text"));
        const result = await index.search(nodes, "text", () => yields.length >= 2);
        expect(result).toBeNull();
        expect(index.folds()).toBeLessThan(500);
    });
});

describe("firstMatchFrom", () => {
    const order = ["a", "b", "c", "d", "e"];

    it("starts at the first match at or after the top row", () => {
        expect(firstMatchFrom(["a", "d"], order, "b")).toBe(1);
        expect(firstMatchFrom(["a", "d"], order, "d")).toBe(1);
        expect(firstMatchFrom(["a", "c", "e"], order, "a")).toBe(0);
    });

    it("wraps to the first match when none is below the top row", () => {
        expect(firstMatchFrom(["a", "b"], order, "d")).toBe(0);
    });

    it("starts at the first match with no top row, or one the document does not hold", () => {
        expect(firstMatchFrom(["b", "c"], order, null)).toBe(0);
        expect(firstMatchFrom(["b", "c"], order, "gone")).toBe(0);
    });

    it("answers -1 with no matches", () => {
        expect(firstMatchFrom([], order, "a")).toBe(-1);
    });
});

describe("topRowId", () => {
    const rows = [
        { nodeId: "a", end: 100 },
        { nodeId: "b", end: 250 },
        { nodeId: "c", end: 400 },
    ];

    it("is the first row whose bottom edge is below the scroll position", () => {
        expect(topRowId(rows, 0)).toBe("a");
        expect(topRowId(rows, 100)).toBe("b");
        expect(topRowId(rows, 399)).toBe("c");
    });

    it("is null past the last row, and with no rows", () => {
        expect(topRowId(rows, 400)).toBeNull();
        expect(topRowId([], 0)).toBeNull();
    });
});
