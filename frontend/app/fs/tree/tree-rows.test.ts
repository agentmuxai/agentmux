// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The tree rows: depth-first order, names by path from the shown folder, and the name helpers (tree-rows.ts). */

import { describe, expect, it } from "vitest";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { sortEntries } from "@/app/view/files/files-sort";
import { flattenTree, isUnderRel, labelOf, parentRel } from "./tree-rows";

const f = (name: string, over: Partial<FsEntry> = {}): FsEntry => ({ name, is_dir: false, is_symlink: false, hidden: false, readonly: false, size: 1, mtime: 0, ...over });
const d = (name: string, over: Partial<FsEntry> = {}): FsEntry => f(name, { is_dir: true, size: undefined, ...over });

const sortByName = (list: readonly FsEntry[]) => sortEntries(list, "name", "asc");
const rowsOf = (top: FsEntry[], expanded: string[], kids: Record<string, FsEntry[]>, keep: (e: FsEntry) => boolean = () => true) =>
    flattenTree(top, new Set(expanded), (rel) => kids[rel], keep, sortByName);

describe("flattenTree", () => {
    const top = [f("z.txt"), d("src"), d("docs")];
    const kids = { src: [f("main.rs"), d("lib")], "src/lib": [f("util.ts")] };

    it("lists the first level, folders first, with no depth", () => {
        const rows = rowsOf(top, [], kids);
        expect(rows.map((r) => [r.entry.name, r.depth, r.expandable, r.expanded])).toEqual([
            ["docs", 0, true, false],
            ["src", 0, true, false],
            ["z.txt", 0, false, false],
        ]);
    });

    it("puts an open folder's rows right under it, named by their path from the shown folder", () => {
        const rows = rowsOf(top, ["src", "src/lib"], kids);
        expect(rows.map((r) => r.entry.name)).toEqual(["docs", "src", "src/lib", "src/lib/util.ts", "src/main.rs", "z.txt"]);
        expect(rows.map((r) => r.depth)).toEqual([0, 0, 1, 2, 1, 0]);
        expect(rows.map((r) => r.label)).toEqual(["docs", "src", "lib", "util.ts", "main.rs", "z.txt"]);
        expect(rows.find((r) => r.entry.name === "src")?.expanded).toBe(true);
    });

    it("marks an open folder whose rows haven't come yet as loading", () => {
        const rows = rowsOf(top, ["docs"], {});
        expect(rows.find((r) => r.entry.name === "docs")).toMatchObject({ expanded: true, loading: true });
        expect(rows).toHaveLength(3);
    });

    it("leaves out what `keep` drops, at every level", () => {
        const rows = rowsOf([f(".env", { hidden: true }), d("src")], ["src"], { src: [f(".git", { hidden: true }), f("a")] }, (e) => !e.hidden);
        expect(rows.map((r) => r.entry.name)).toEqual(["src", "src/a"]);
    });

    it("never opens a link to a folder in place", () => {
        const rows = rowsOf([d("link", { is_symlink: true })], ["link"], { link: [f("x")] });
        expect(rows).toMatchObject([{ expandable: false, expanded: false }]);
    });

    it("ignores an open folder that is no longer there", () => {
        expect(rowsOf([f("a")], ["gone"], { gone: [f("x")] }).map((r) => r.entry.name)).toEqual(["a"]);
    });
});

describe("row names", () => {
    it("splits a name into its folder and its own name", () => {
        expect(parentRel("src/lib/a.ts")).toBe("src/lib");
        expect(parentRel("a.ts")).toBe("");
        expect(labelOf("src/lib/a.ts")).toBe("a.ts");
        expect(labelOf("a.ts")).toBe("a.ts");
    });

    it("says whether a row is inside a folder, or is it", () => {
        expect(isUnderRel("src/lib", "src")).toBe(true);
        expect(isUnderRel("src", "src")).toBe(true);
        expect(isUnderRel("src2/a", "src")).toBe(false);
    });
});
