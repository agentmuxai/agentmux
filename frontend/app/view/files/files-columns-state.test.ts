// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The columns view's state: the chain of opened folders, the active column, and the moves between (files-columns-state.ts). */

import type { FsEntry } from "@/types/rpc/FsEntry";
import { createRoot, createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import { ColumnsState, type ColumnsHost } from "./files-columns-state";
import { EMPTY_SELECTION, type Selection } from "./files-selection";
import { sortEntries } from "./files-sort";

const f = (name: string, over: Partial<FsEntry> = {}): FsEntry => ({ name, is_dir: false, is_symlink: false, hidden: false, readonly: false, size: 1, mtime: 0, ...over });
const d = (name: string, over: Partial<FsEntry> = {}): FsEntry => f(name, { is_dir: true, size: undefined, ...over });

/** A fake model: the shown folder holds `top`; `disk` has what each opened folder lists. */
function setup(top: FsEntry[], disk: Record<string, FsEntry[]>, opts: { consent?: string } = {}) {
    const [expanded, setExpanded] = createSignal<ReadonlySet<string>>(new Set());
    const [kids, setKids] = createSignal<ReadonlyMap<string, FsEntry[]>>(new Map());
    const [selection, setSelection] = createSignal<Selection>(EMPTY_SELECTION);
    const [filter, setFilter] = createSignal("");
    const [hidden, setHidden] = createSignal(false);
    const log: string[] = [];
    const host: ColumnsHost = {
        on: () => true,
        expanded,
        setExpanded: (n) => setExpanded(n),
        kids,
        setKids: (n) => setKids(n),
        rawEntries: () => top,
        shows: (e) => hidden() || !e.hidden,
        sort: (list) => sortEntries(list, "name", "asc"),
        filter,
        selection,
        setSelection,
        reveal: (name) => log.push(`reveal ${name}`),
        // Listing is instant here.
        open: (rel) => {
            log.push(`open ${rel}`);
            setKids(new Map(kids()).set(rel, disk[rel] ?? []));
        },
        close: (rel) => log.push(`close ${rel}`),
        listable: (rel) => rel !== opts.consent,
        maxDepth: 40,
    };
    const cols = new ColumnsState(host);
    const pick = (...names: string[]) => setSelection({ names: new Set(names), focus: names[0] ?? null, anchor: names[0] ?? null });
    return { cols, pick, selection, setFilter, setHidden, expanded, log };
}

const names = (cols: ColumnsState, i: number) => cols.columns()[i]?.entries.map((e) => e.name);

describe("ColumnsState", () => {
    it("is one column of the shown folder until a folder is chosen", () =>
        createRoot((dispose) => {
            const s = setup([f("b.txt"), d("src")], {});
            expect(s.cols.columns()).toHaveLength(1);
            expect(names(s.cols, 0)).toEqual(["src", "b.txt"]);
            expect(s.cols.activeColumn()).toBe(0);
            dispose();
        }));

    it("opens the folder chosen as the next column, named by path, and drops it for a file", () =>
        createRoot((dispose) => {
            const s = setup([f("b.txt"), d("src")], { src: [f("main.rs"), d("lib")], "src/lib": [f("util.ts")] });
            s.pick("src");
            s.cols.sync();
            expect(s.cols.chain()).toEqual(["src"]);
            expect(names(s.cols, 1)).toEqual(["src/lib", "src/main.rs"]);
            expect(s.cols.onPath("src")).toBe(true);
            s.cols.focusColumn(1);
            s.pick("src/lib");
            s.cols.sync();
            expect(names(s.cols, 2)).toEqual(["src/lib/util.ts"]);
            // A file in the first column ends the chain after it.
            s.cols.focusColumn(0);
            s.pick("b.txt");
            s.cols.sync();
            expect(s.cols.chain()).toEqual([]);
            expect(s.log).toContain("close src");
            expect(s.log).toContain("close src/lib");
            dispose();
        }));

    it("leaves a folder alone that may not be listed, and several chosen rows open nothing", () =>
        createRoot((dispose) => {
            const s = setup([d("Documents"), d("src")], { src: [f("a")] }, { consent: "Documents" });
            s.pick("Documents");
            s.cols.sync();
            expect(s.cols.chain()).toEqual([]);
            s.pick("Documents", "src");
            s.cols.sync();
            expect(s.cols.chain()).toEqual([]);
            dispose();
        }));

    it("moves into the next column with Right and back with Left", () =>
        createRoot((dispose) => {
            const s = setup([d("src")], { src: [d("lib"), f("main.rs")], "src/lib": [f("util.ts")] });
            s.pick("src");
            s.cols.arrow("right");
            expect(s.selection().focus).toBe("src/lib");
            expect(s.cols.activeColumn()).toBe(1);
            expect(s.cols.chain()).toEqual(["src", "src/lib"]);
            s.cols.arrow("right");
            expect(s.selection().focus).toBe("src/lib/util.ts");
            expect(s.cols.activeColumn()).toBe(2);
            s.cols.arrow("left");
            expect(s.selection().focus).toBe("src/lib");
            s.cols.arrow("left");
            expect(s.selection().focus).toBe("src");
            expect(s.cols.activeColumn()).toBe(0);
            expect(s.cols.chain()).toEqual(["src"]);
            // Nothing further left.
            s.cols.arrow("left");
            expect(s.selection().focus).toBe("src");
            dispose();
        }));

    it("does not step right from a file or into an empty folder", () =>
        createRoot((dispose) => {
            const s = setup([f("a.txt"), d("empty")], { empty: [] });
            s.pick("a.txt");
            s.cols.arrow("right");
            expect(s.cols.activeColumn()).toBe(0);
            s.pick("empty");
            s.cols.arrow("right");
            expect(s.cols.activeColumn()).toBe(0);
            expect(s.selection().focus).toBe("empty");
            dispose();
        }));

    it("hides hidden files unless shown, and filters the active column only", () =>
        createRoot((dispose) => {
            const s = setup([f(".env", { hidden: true }), d("src"), f("note.md")], { src: [f("note-2.md"), f("x.ts")] });
            expect(names(s.cols, 0)).toEqual(["src", "note.md"]);
            s.setHidden(true);
            expect(names(s.cols, 0)).toEqual(["src", ".env", "note.md"]);
            s.pick("src");
            s.cols.sync();
            s.setFilter("note");
            // The first column is the active one and is filtered; the second shows all it holds.
            expect(s.cols.entries().map((e) => e.name)).toEqual(["note.md"]);
            expect(names(s.cols, 1)).toEqual(["src/note-2.md", "src/x.ts"]);
            dispose();
        }));

    it("clicking another column makes it the active one and lets go of the selection", () =>
        createRoot((dispose) => {
            const s = setup([d("src")], { src: [f("a")] });
            s.pick("src");
            s.cols.sync();
            s.cols.focusColumn(1);
            expect(s.cols.activeColumn()).toBe(1);
            expect(s.selection().names.size).toBe(0);
            s.cols.reset();
            expect(s.cols.activeColumn()).toBe(0);
            dispose();
        }));
});
