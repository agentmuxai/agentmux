// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Finding by name under the shown folder: the fuzzy score, the ranking, and the bounded walk (files-find.ts). */

import { describe, expect, it } from "vitest";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { crawlFolders, DEFAULT_CRAWL, fuzzyScore, rankMatches, whereOf } from "./files-find";

const f = (name: string, over: Partial<FsEntry> = {}): FsEntry => ({ name, is_dir: false, is_symlink: false, hidden: false, readonly: false, size: 1, mtime: 0, ...over });
const d = (name: string, over: Partial<FsEntry> = {}): FsEntry => f(name, { is_dir: true, size: undefined, ...over });

describe("fuzzyScore", () => {
    it("needs the letters in order, case aside, and says null when they are not", () => {
        expect(fuzzyScore("mrs", "src/main.rs")).not.toBeNull();
        expect(fuzzyScore("MAIN", "src/main.rs")).not.toBeNull();
        expect(fuzzyScore("smr", "src/main.rs")).not.toBeNull();
        expect(fuzzyScore("nms", "src/main.rs")).toBeNull();
        expect(fuzzyScore("zzz", "src/main.rs")).toBeNull();
    });

    it("matches nothing for an empty query", () => {
        expect(fuzzyScore("", "a")).toBeNull();
        expect(fuzzyScore("   ", "a")).toBeNull();
    });

    it("prefers the whole query in the file's own name over letters spread over folders", () => {
        const inName = fuzzyScore("main", "docs/main.md")!;
        const spread = fuzzyScore("main", "m/a/i/n/x.txt")!;
        expect(inName).toBeGreaterThan(spread);
    });

    it("prefers the start of the name, and the start of a word, over the middle", () => {
        expect(fuzzyScore("ma", "src/main.rs")!).toBeGreaterThan(fuzzyScore("ma", "src/format.rs")!);
        expect(fuzzyScore("fb", "src/foo-bar.ts")!).toBeGreaterThan(fuzzyScore("fb", "src/fabric.ts")!);
        expect(fuzzyScore("fb", "src/fooBar.ts")!).toBeGreaterThan(fuzzyScore("fb", "src/fabric.ts")!);
    });

    it("prefers letters that follow each other, and a shorter path on a tie", () => {
        expect(fuzzyScore("abc", "x/abc.ts")!).toBeGreaterThan(fuzzyScore("abc", "x/a-b-c.ts")!);
        expect(fuzzyScore("a", "a.ts")!).toBeGreaterThan(fuzzyScore("a", "a.tsx")!);
    });
});

describe("rankMatches", () => {
    const index = [f("src/main.rs"), f("docs/maintaining.md"), f("main.ts"), f("readme.md"), f("lib/domain.rs")];

    it("lists the matches best first and leaves out the rest", () => {
        const names = rankMatches(index, "main", 10).map((e) => e.name);
        expect(names[0]).toBe("main.ts");
        expect(names).toContain("src/main.rs");
        expect(names).toContain("lib/domain.rs");
        expect(names).not.toContain("readme.md");
    });

    it("keeps the best `limit`", () => {
        expect(rankMatches(index, "m", 2)).toHaveLength(2);
        expect(rankMatches(index, "zzz", 5)).toEqual([]);
    });
});

describe("whereOf", () => {
    it("is the folder a path is in, empty at the first level", () => {
        expect(whereOf("src/lib/a.ts")).toBe("src/lib");
        expect(whereOf("a.ts")).toBe("");
    });
});

/** A fake disk: folder path from the shown folder ("" for it) to its entries. */
const disk = (tree: Record<string, FsEntry[]>) => {
    const listed: string[] = [];
    const list = async (rel: string) => {
        listed.push(rel);
        return tree[rel] ?? null;
    };
    return { list, listed };
};

describe("crawlFolders", () => {
    const run = async (tree: Record<string, FsEntry[]>, over: { hidden?: boolean; aborted?: () => boolean; limits?: Partial<typeof DEFAULT_CRAWL> } = {}) => {
        const { list, listed } = disk(tree);
        const got: string[] = [];
        const res = await crawlFolders(list, (b) => got.push(...b.map((e) => e.name)), over.aborted ?? (() => false), over.hidden ?? false, { ...DEFAULT_CRAWL, ...over.limits });
        return { res, got, listed };
    };

    it("names every entry by its path from the shown folder, folders and files alike", async () => {
        const { res, got } = await run({
            "": [d("src"), f("a.md")],
            src: [f("main.rs"), d("lib")],
            "src/lib": [f("util.ts")],
        });
        expect([...got].sort()).toEqual(["a.md", "src", "src/lib", "src/lib/util.ts", "src/main.rs"]);
        expect(res).toMatchObject({ end: "done", folders: 3, entries: 5 });
    });

    it("never walks into .git or node_modules, a link to a folder, or a hidden folder unless asked", async () => {
        const tree = {
            "": [d(".git"), d("node_modules"), d("link", { is_symlink: true }), d(".config", { hidden: true }), d("src")],
            ".git": [f("HEAD")],
            node_modules: [f("x.js")],
            link: [f("loop")],
            ".config": [f("settings.json")],
            src: [f("a.ts")],
        };
        const off = await run(tree);
        expect(off.listed.sort()).toEqual(["", "src"]);
        expect(off.got).not.toContain(".config");
        const on = await run(tree, { hidden: true });
        expect(on.listed).toContain(".config");
        expect(on.got).toContain(".config/settings.json");
        expect(on.listed).not.toContain(".git/HEAD");
        expect(on.listed).not.toContain(".git");
    });

    it("skips a folder it can't read and goes on", async () => {
        const { got, res } = await run({ "": [d("denied"), d("ok")], ok: [f("a")] });
        expect(got).toContain("ok/a");
        expect(res.end).toBe("done");
    });

    it("stops at the folder limit and says so", async () => {
        const tree: Record<string, FsEntry[]> = { "": [d("a"), d("b"), d("c"), d("d")] };
        for (const n of ["a", "b", "c", "d"]) tree[n] = [f("x")];
        const { res } = await run(tree, { limits: { maxFolders: 3, parallel: 1 } });
        expect(res.end).toBe("limit");
        expect(res.folders).toBe(3);
    });

    it("stops at the entry limit and at the time limit", async () => {
        const many = Array.from({ length: 10 }, (_, i) => f(`f${i}`));
        const byEntries = await run({ "": [d("a"), ...many], a: many }, { limits: { maxEntries: 5, parallel: 1 } });
        expect(byEntries.res.end).toBe("limit");
        let t = 0;
        const { list } = disk({ "": [d("a")], a: [f("x")] });
        const slow = await crawlFolders(list, () => {}, () => false, false, { ...DEFAULT_CRAWL, maxMs: 100, parallel: 1 }, () => (t += 60));
        expect(slow.end).toBe("limit");
    });

    it("stops when told to", async () => {
        let calls = 0;
        const { res } = await run({ "": [d("a"), d("b")], a: [f("x")], b: [f("y")] }, { aborted: () => ++calls > 2, limits: { parallel: 1 } });
        expect(res.end).toBe("aborted");
    });

    it("lists the folders under a deep chain with several workers", async () => {
        const tree: Record<string, FsEntry[]> = { "": [d("a")], a: [d("b")], "a/b": [d("c")], "a/b/c": [f("z")] };
        const { got, res } = await run(tree, { limits: { parallel: 4 } });
        expect(got).toContain("a/b/c/z");
        expect(res.end).toBe("done");
    });
});
