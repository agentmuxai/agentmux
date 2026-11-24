// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The columns view of the Files pane (docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md section 5.2, "Columns"): a strip
 * of lists, the shown folder first, then the folder chosen in each column, one column per level. It is the tree view's
 * state drawn another way: a row is named by its path from the shown folder (`src/lib/a.ts`), and the folders opened
 * are one chain, so selection, rename, trash, copy and drag work as they do in the tree. The model owns the folders'
 * listings and watches; this holds which column is the one the keys and the selection are in, and the moves between.
 */

import type { FsEntry } from "@/types/rpc/FsEntry";
import { batch, createMemo, createSignal } from "solid-js";
import { type Selection } from "./files-selection";

/** One column: the folder it lists (its path from the shown folder, "" for the shown folder) and its rows, named by path. */
export interface ColumnView {
    dir: string;
    entries: FsEntry[];
}

/** What the columns need from the model. */
export interface ColumnsHost {
    on(): boolean;
    expanded(): ReadonlySet<string>;
    setExpanded(next: Set<string>): void;
    kids(): ReadonlyMap<string, FsEntry[]>;
    setKids(next: Map<string, FsEntry[]>): void;
    rawEntries(): FsEntry[];
    shows(e: FsEntry): boolean;
    sort(list: FsEntry[]): FsEntry[];
    filter(): string;
    selection(): Selection;
    setSelection(next: Selection): void;
    reveal(name: string): void;
    /** Starts watching and listing an opened folder / stops watching a closed one. */
    open(rel: string): void;
    close(rel: string): void;
    /** Whether the folder may be listed on its own (macOS asks before some). */
    listable(rel: string): boolean;
    maxDepth: number;
}

const depthOf = (rel: string): number => rel.split("/").length;

export class ColumnsState {
    private readonly active: () => number;
    private readonly setActive: (n: number) => void;
    /** The folders opened, outermost first: column n + 1 lists `chain()[n]`. */
    readonly chain: () => string[];
    readonly columns: () => ColumnView[];
    /** The column the keys and the selection are in. */
    readonly activeColumn: () => number;
    /** The rows of the active column, filtered by what is typed in the filter box. */
    readonly entries: () => FsEntry[];

    constructor(private readonly host: ColumnsHost) {
        [this.active, this.setActive] = createSignal(0);
        this.chain = createMemo(() => (host.on() ? [...host.expanded()].sort((a, b) => depthOf(a) - depthOf(b)) : []));
        this.columns = createMemo(() => {
            if (!host.on()) return [];
            const named = (dir: string, list: readonly FsEntry[]): FsEntry[] =>
                host.sort(list.filter((e) => host.shows(e)).map((e) => (dir === "" ? e : { ...e, name: `${dir}/${e.name}` })));
            return ["", ...this.chain()].map((dir) => ({ dir, entries: named(dir, dir === "" ? host.rawEntries() : (host.kids().get(dir) ?? [])) }));
        });
        this.activeColumn = () => Math.min(this.active(), this.chain().length);
        this.entries = createMemo(() => {
            const needle = host.filter().trim().toLowerCase();
            const rows = this.columns()[this.activeColumn()]?.entries ?? [];
            return needle === "" ? rows : rows.filter((e) => e.name.slice(e.name.lastIndexOf("/") + 1).toLowerCase().includes(needle));
        });
    }

    /** True for a folder row that is open, so drawn as the way the columns lead. */
    readonly onPath = (name: string): boolean => this.host.expanded().has(name);

    /** Back to one column (the folder was left, or the view was). */
    reset(): void {
        this.setActive(0);
    }

    /** Makes column `col` the one the keys work in; what was selected in another column is let go. */
    focusColumn(col: number): void {
        if (col === this.activeColumn()) return;
        batch(() => {
            this.setActive(col);
            this.host.setSelection({ names: new Set(), focus: null, anchor: null });
        });
    }

    /** Opens the folder chosen in the active column as the next column, and drops the columns after it. */
    sync(): void {
        if (!this.host.on()) return;
        const want = this.chain().slice(0, this.activeColumn());
        const names = this.host.selection().names;
        const picked = names.size === 1 ? this.entries().find((e) => e.name === [...names][0]) : undefined;
        if (picked?.is_dir && !picked.is_symlink && want.length < this.host.maxDepth && this.host.listable(picked.name)) want.push(picked.name);
        this.setChain(want);
    }

    private setChain(want: string[]): void {
        const have = this.host.expanded();
        const next = new Set(want);
        if (next.size === have.size && want.every((rel) => have.has(rel))) return;
        const kids = new Map(this.host.kids());
        for (const rel of have) {
            if (next.has(rel)) continue;
            kids.delete(rel);
            this.host.close(rel);
        }
        batch(() => {
            this.host.setExpanded(next);
            this.host.setKids(kids);
        });
        for (const rel of want) if (!have.has(rel)) this.host.open(rel);
    }

    /** Right steps into the folder chosen, to its first row; Left steps back to the folder this column lists. */
    arrow(dir: "right" | "left"): void {
        const a = this.activeColumn();
        let target: string | undefined;
        let col = a;
        if (dir === "right") {
            this.sync();
            target = this.columns()[a + 1]?.entries[0]?.name;
            col = a + 1;
        } else if (a > 0) {
            target = this.chain()[a - 1];
            col = a - 1;
        }
        if (target === undefined) return;
        const name = target;
        batch(() => {
            this.setActive(col);
            this.host.setSelection({ names: new Set([name]), focus: name, anchor: name });
            this.host.reveal(name);
        });
        this.sync();
    }
}
