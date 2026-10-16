// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { DirEntry } from "@/app/store/rpc-api";
import { describe, expect, it } from "vitest";
import {
    entryNamed,
    isOpenFile,
    rootHolding,
    segmentsBelow,
    treeMove,
    TreeTypeAhead,
    visibleRows,
    type TreeRow,
    type TreeSource,
} from "./tree-keyboard";

const dir = (name: string): DirEntry => ({ name, is_dir: true, is_symlink: false }) as DirEntry;
const file = (name: string): DirEntry => ({ name, is_dir: false, is_symlink: false }) as DirEntry;

/** /home/a holding src/ (main.ts, util.ts), docs/ (closed), .git/ and README.md. */
function source(expanded: string[] = ["/home/a", "/home/a/src"]): TreeSource {
    const listings: Record<string, DirEntry[]> = {
        "/home/a": [dir(".git"), dir("docs"), dir("src"), file("README.md")],
        "/home/a/src": [file("main.ts"), file("util.ts")],
        "/home/a/docs": [file("guide.md")],
    };
    return {
        roots: [{ name: "a", path: "/home/a" }],
        isExpanded: (p) => expanded.includes(p),
        entries: (p) => listings[p],
    };
}

const paths = (rows: TreeRow[]) => rows.map((r) => r.path);

describe("visibleRows", () => {
    it("lists the roots and each open folder's contents, in display order", () => {
        expect(paths(visibleRows(source(), false))).toEqual([
            "/home/a",
            "/home/a/docs",
            "/home/a/src",
            "/home/a/src/main.ts",
            "/home/a/src/util.ts",
            "/home/a/README.md",
        ]);
    });

    it("shows hidden entries only when asked", () => {
        expect(paths(visibleRows(source(), true))).toContain("/home/a/.git");
    });

    it("records each row's depth, parent and state", () => {
        const main = visibleRows(source(), false).find((r) => r.name === "main.ts")!;
        expect(main).toMatchObject({ depth: 2, parent: "/home/a/src", isDir: false, expanded: false });
        expect(visibleRows(source(), false)[0]).toMatchObject({ depth: 0, parent: null, expanded: true });
    });
});

describe("treeMove", () => {
    const rows = visibleRows(source(), false);
    const move = (selected: string | null, key: string, page = 10) => treeMove(rows, selected, key, page);

    it("moves ↓ and ↑ across expanded folders and stops at the ends", () => {
        expect(move("/home/a/src", "ArrowDown")).toEqual({ kind: "select", path: "/home/a/src/main.ts" });
        expect(move("/home/a/src/util.ts", "ArrowDown")).toEqual({ kind: "select", path: "/home/a/README.md" });
        expect(move("/home/a/README.md", "ArrowUp")).toEqual({ kind: "select", path: "/home/a/src/util.ts" });
        expect(move("/home/a/README.md", "ArrowDown")).toEqual({ kind: "select", path: "/home/a/README.md" });
        expect(move("/home/a", "ArrowUp")).toEqual({ kind: "select", path: "/home/a" });
    });

    it("→ opens a closed folder, then steps into it; nothing on a file", () => {
        expect(move("/home/a/docs", "ArrowRight")).toEqual({ kind: "expand", path: "/home/a/docs" });
        expect(move("/home/a/src", "ArrowRight")).toEqual({ kind: "select", path: "/home/a/src/main.ts" });
        expect(move("/home/a/src/main.ts", "ArrowRight")).toEqual({ kind: "none" });
    });

    it("→ on an open folder whose contents haven't loaded does nothing", () => {
        const loading = visibleRows({ ...source(["/home/a", "/home/a/docs"]), entries: (p) => (p === "/home/a" ? source().entries(p) : undefined) }, false);
        expect(treeMove(loading, "/home/a/docs", "ArrowRight", 10)).toEqual({ kind: "none" });
    });

    it("← closes an open folder, else goes to the parent", () => {
        expect(move("/home/a/src", "ArrowLeft")).toEqual({ kind: "collapse", path: "/home/a/src" });
        expect(move("/home/a/src/util.ts", "ArrowLeft")).toEqual({ kind: "select", path: "/home/a/src" });
        expect(move("/home/a/docs", "ArrowLeft")).toEqual({ kind: "select", path: "/home/a" });
        // A collapsed root has nowhere to go.
        expect(treeMove(visibleRows(source([]), false), "/home/a", "ArrowLeft", 10)).toEqual({ kind: "none" });
    });

    it("Home and End go to the first and last rows; Page keys move a screenful", () => {
        expect(move("/home/a/src", "Home")).toEqual({ kind: "select", path: "/home/a" });
        expect(move("/home/a/src", "End")).toEqual({ kind: "select", path: "/home/a/README.md" });
        expect(move("/home/a", "PageDown", 2)).toEqual({ kind: "select", path: "/home/a/src" });
        expect(move("/home/a/src/util.ts", "PageUp", 2)).toEqual({ kind: "select", path: "/home/a/src" });
        expect(move("/home/a/src", "PageDown", 50)).toEqual({ kind: "select", path: "/home/a/README.md" });
    });

    it("starts at the first row when nothing is selected (End at the last)", () => {
        expect(move(null, "ArrowDown")).toEqual({ kind: "select", path: "/home/a" });
        expect(move("/gone", "ArrowUp")).toEqual({ kind: "select", path: "/home/a" });
        expect(move(null, "End")).toEqual({ kind: "select", path: "/home/a/README.md" });
    });

    it("leaves other keys alone", () => {
        expect(move("/home/a", "Enter")).toBeNull();
        expect(move("/home/a", "a")).toBeNull();
    });
});

describe("TreeTypeAhead", () => {
    const rows = visibleRows(source(), false);

    it("selects the next row starting with what was typed, wrapping round", () => {
        const t = new TreeTypeAhead();
        expect(t.next("u", rows, "/home/a", 1000)).toBe("/home/a/src/util.ts");
        // After a pause a new prefix starts, searching past the selected row.
        expect(t.next("r", rows, "/home/a/src/util.ts", 2000)).toBe("/home/a/README.md");
        expect(t.next("a", rows, "/home/a/README.md", 3000)).toBe("/home/a");
    });

    it("builds a prefix from keys typed close together", () => {
        const t = new TreeTypeAhead();
        expect(t.next("s", rows, null, 1000)).toBe("/home/a/src");
        expect(t.next("r", rows, "/home/a/src", 1100)).toBe("/home/a/src");
        expect(t.active(1200)).toBe(true);
        expect(t.active(2000)).toBe(false);
    });

    it("steps through rows starting with a repeated letter", () => {
        const t = new TreeTypeAhead();
        expect(t.next("m", rows, null, 1000)).toBe("/home/a/src/main.ts");
        expect(t.next("d", rows, "/home/a/src/main.ts", 3000)).toBe("/home/a/docs");
        const u = new TreeTypeAhead();
        const twoS = [...rows, { ...rows[0], path: "/x/spec", name: "spec" }];
        expect(u.next("s", twoS, null, 1000)).toBe("/home/a/src");
        expect(u.next("s", twoS, "/home/a/src", 1100)).toBe("/x/spec");
    });

    it("finds nothing when no name matches", () => {
        expect(new TreeTypeAhead().next("z", rows, null, 1000)).toBeNull();
    });
});

describe("the open file", () => {
    it("matches a row whatever its separators and its drive letter's case", () => {
        expect(isOpenFile("C:\\Users\\a\\x.ts", "c:/Users/a/x.ts")).toBe(true);
        expect(isOpenFile("C:\\Users\\a\\x.ts", "C:\\Users\\a\\x.ts")).toBe(true);
        expect(isOpenFile("/home/a/x.ts", "/home/a/x.ts")).toBe(true);
    });

    it("tells different files apart, and nothing is open for an empty path", () => {
        expect(isOpenFile("C:\\Users\\a\\x.ts", "c:/Users/a/y.ts")).toBe(false);
        // Case counts outside Windows.
        expect(isOpenFile("/home/a/X.ts", "/home/a/x.ts")).toBe(false);
        expect(isOpenFile("", "")).toBe(false);
    });

    it("finds the root holding it, the deepest when roots nest", () => {
        expect(rootHolding(["C:\\Users\\a", "D:\\"], "c:/Users/a/src/x.ts")).toBe("C:\\Users\\a");
        expect(rootHolding(["C:\\Users\\a", "D:\\"], "d:/work/x.ts")).toBe("D:\\");
        expect(rootHolding(["/home/a", "/"], "/home/a/x.ts")).toBe("/home/a");
        expect(rootHolding(["/home/a", "/"], "/etc/hosts")).toBe("/");
        expect(rootHolding(["C:\\Users\\a"], "e:/x.ts")).toBeNull();
        // Not a sibling that only shares a prefix.
        expect(rootHolding(["/home/a"], "/home/ab/x.ts")).toBeNull();
    });

    it("lists the names from the root down to it", () => {
        expect(segmentsBelow("C:\\Users\\a", "c:/Users/a/src/x.ts")).toEqual(["src", "x.ts"]);
        expect(segmentsBelow("D:\\", "d:/work/x.ts")).toEqual(["work", "x.ts"]);
        expect(segmentsBelow("/", "/etc/hosts")).toEqual(["etc", "hosts"]);
    });

    it("finds an entry by name, ignoring case only where told to", () => {
        const entries = [file("Readme.md"), file("readme.txt")];
        expect(entryNamed(entries, "Readme.md", false)?.name).toBe("Readme.md");
        expect(entryNamed(entries, "README.md", false)).toBeUndefined();
        expect(entryNamed(entries, "README.md", true)?.name).toBe("Readme.md");
    });
});
