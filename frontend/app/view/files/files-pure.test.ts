// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { baseName, crumbsOf, isWithin, joinPath, nameProblem, normalizePath, parentOf, samePath, stemLength } from "./files-path";
import { clickRow, EMPTY_SELECTION, moveFocus, pruneSelection, selectAll, toggleFocused } from "./files-selection";
import { sortEntries } from "./files-sort";
import { TypeAhead } from "./typeahead";

describe("files-path", () => {
    it("breaks a Windows drive path into crumbs, the drive first", () => {
        expect(crumbsOf("C:\\Users\\a")).toEqual([
            { label: "C:", path: "C:\\" },
            { label: "Users", path: "C:\\Users" },
            { label: "a", path: "C:\\Users\\a" },
        ]);
        expect(crumbsOf("C:\\")).toEqual([{ label: "C:", path: "C:\\" }]);
    });

    it("treats a UNC share as the root", () => {
        expect(crumbsOf("\\\\srv\\share\\x")).toEqual([
            { label: "\\\\srv\\share", path: "\\\\srv\\share\\" },
            { label: "x", path: "\\\\srv\\share\\x" },
        ]);
        expect(parentOf("\\\\srv\\share\\")).toBeNull();
    });

    it("breaks a POSIX path into crumbs, / first", () => {
        expect(crumbsOf("/home/a")).toEqual([
            { label: "/", path: "/" },
            { label: "home", path: "/home" },
            { label: "a", path: "/home/a" },
        ]);
        expect(parentOf("/home/a")).toBe("/home");
        expect(parentOf("/home")).toBe("/");
        expect(parentOf("/")).toBeNull();
    });

    it("joins with the folder's own separator", () => {
        expect(joinPath("C:\\Users", "a.txt")).toBe("C:\\Users\\a.txt");
        expect(joinPath("C:\\", "a.txt")).toBe("C:\\a.txt");
        expect(joinPath("/home", "a")).toBe("/home/a");
        expect(joinPath("/", "a")).toBe("/a");
        expect(baseName("C:\\Users\\a.txt")).toBe("a.txt");
    });

    it("compares Windows paths without case, POSIX paths with it", () => {
        expect(samePath("C:\\Users\\A\\", "c:/users/a")).toBe(true);
        expect(samePath("/home/A", "/home/a")).toBe(false);
        expect(isWithin("C:\\Users\\a\\Documents\\x", "C:\\Users\\a\\Documents")).toBe(true);
        expect(isWithin("C:\\Users\\a\\Documents2", "C:\\Users\\a\\Documents")).toBe(false);
    });

    it.each([
        ["", true, "A name can't be empty."],
        ["..", false, '".." isn\'t a valid name.'],
        ["a/b", false, "A name can't contain / or \\."],
        ["a:b", true, 'A name can\'t contain < > : " | ? * or control characters.'],
        ["a.", true, "On Windows a name can't end with a dot or a space."],
        ["CON", true, '"CON" is reserved by Windows.'],
        ["nul.txt", true, '"nul.txt" is reserved by Windows.'],
        ["a:b", false, null],
        ["console.txt", true, null],
    ])("nameProblem(%j, windows=%s)", (name, windows, problem) => {
        expect(nameProblem(name, windows)).toBe(problem);
    });

    it("normalizes ~, . and .. before comparing (muxreview on #4201)", () => {
        expect(normalizePath("~/Documents", "/Users/a")).toBe("/Users/a/Documents");
        expect(normalizePath("/Users/a/Desktop/../Documents/./x", "/Users/a")).toBe("/Users/a/Documents/x");
        expect(normalizePath("C:\\Users\\a\\..\\b", "")).toBe("C:\\Users\\b");
        expect(normalizePath("/..", "")).toBe("/");
    });

    it("selects the stem of a file name for a rename", () => {
        expect(stemLength("report.final.pdf", false)).toBe("report.final".length);
        expect(stemLength(".gitignore", false)).toBe(".gitignore".length);
        expect(stemLength("src.old", true)).toBe("src.old".length);
    });
});

const e = (name: string, over: Partial<FsEntry> = {}): FsEntry => ({
    name,
    is_dir: false,
    is_symlink: false,
    hidden: false,
    readonly: false,
    ...over,
});

describe("sortEntries", () => {
    it("puts folders first and compares names naturally", () => {
        const sorted = sortEntries([e("file10"), e("b", { is_dir: true }), e("file2"), e("A", { is_dir: true })], "name", "asc");
        expect(sorted.map((x) => x.name)).toEqual(["A", "b", "file2", "file10"]);
    });

    it("keeps folders first when the column runs the other way", () => {
        const sorted = sortEntries([e("a"), e("z", { is_dir: true }), e("b")], "name", "desc");
        expect(sorted.map((x) => x.name)).toEqual(["z", "b", "a"]);
    });

    it("sorts by size and modified, ties by name", () => {
        const list = [e("c", { size: 5, mtime: 2 }), e("a", { size: 5, mtime: 3 }), e("b", { size: 1, mtime: 1 })];
        expect(sortEntries(list, "size", "asc").map((x) => x.name)).toEqual(["b", "a", "c"]);
        expect(sortEntries(list, "modified", "desc").map((x) => x.name)).toEqual(["a", "c", "b"]);
        expect(sortEntries([e("x.ts"), e("y.md"), e("z")], "kind", "asc").map((x) => x.name)).toEqual(["z", "y.md", "x.ts"]);
    });
});

describe("selection", () => {
    const order = ["a", "b", "c", "d", "e"];

    it("plain click selects one; Ctrl toggles; Shift selects the range from the anchor", () => {
        let s = clickRow(EMPTY_SELECTION, order, "b", { toggle: false, range: false });
        expect([...s.names]).toEqual(["b"]);
        s = clickRow(s, order, "d", { toggle: false, range: true });
        expect([...s.names]).toEqual(["b", "c", "d"]);
        s = clickRow(s, order, "c", { toggle: true, range: false });
        expect([...s.names].sort()).toEqual(["b", "d"]);
        expect(s.anchor).toBe("c");
    });

    it("arrow keys move the selection; Shift extends from the anchor; Ctrl moves only focus", () => {
        let s = moveFocus(EMPTY_SELECTION, order, { delta: 1 }, { extend: false, focusOnly: false });
        expect(s.focus).toBe("a");
        s = moveFocus(s, order, { delta: 2 }, { extend: true, focusOnly: false });
        expect([...s.names]).toEqual(["a", "b", "c"]);
        s = moveFocus(s, order, { delta: 1 }, { extend: false, focusOnly: true });
        expect(s.focus).toBe("d");
        expect([...s.names]).toEqual(["a", "b", "c"]);
        s = toggleFocused(s);
        expect([...s.names]).toEqual(["a", "b", "c", "d"]);
        s = moveFocus(s, order, { to: 99 }, { extend: false, focusOnly: false });
        expect(s.focus).toBe("e");
    });

    it("selects all", () => {
        expect([...selectAll(EMPTY_SELECTION, order).names]).toEqual(order);
    });

    it("keeps focus near where it was when its row goes away", () => {
        const s = { names: new Set(["c", "d"]), focus: "c", anchor: "c" };
        const pruned = pruneSelection(s, order, ["a", "b", "d", "e"]);
        expect(pruned).toEqual({ names: new Set(["d"]), focus: "d", anchor: "d" });
    });
});

describe("TypeAhead", () => {
    const order = ["alpha", "apple", "beta", "banana", "bravo"];

    it("jumps by prefix, growing while keys come fast", () => {
        const t = new TypeAhead();
        expect(t.next("b", order, null, 0)).toBe("beta");
        expect(t.next("r", order, "beta", 100)).toBe("bravo");
    });

    it("cycles through names with a repeated letter, and starts over after a pause", () => {
        const t = new TypeAhead();
        expect(t.next("a", order, null, 0)).toBe("alpha");
        expect(t.next("a", order, "alpha", 100)).toBe("apple");
        expect(t.next("a", order, "apple", 200)).toBe("alpha");
        expect(t.next("b", order, "alpha", 1000)).toBe("beta");
        expect(t.active(1100)).toBe(true);
        expect(t.active(2000)).toBe(false);
    });
});
