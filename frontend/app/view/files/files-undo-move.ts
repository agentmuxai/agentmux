// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Undo for a move (docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §7.3). srv's move jobs don't say where each item
 * ended up, and a conflict answer (Replace, Keep both, a merged folder) changes that, so a move is undoable only when
 * the answer is certain: every item went to `<destination>/<its own name>` and nothing by that name was there before.
 * Checked before the job starts (the names in the destination) and again when undoing (the items are where the move
 * left them, and the places they came from are free), so Undo never moves a file that merely shares a name.
 */

import { baseName, joinPath, parentOf, samePath } from "./files-path";

/** One moved item: where it was, and where the move put it. */
export interface MovedItem {
    from: string;
    to: string;
}

/** A move that can be undone for certain: the items, and the names the destination held before it started. */
export interface UndoPlan {
    items: MovedItem[];
    destDir: string;
    before: ReadonlySet<string>;
}

/**
 * The items a move will leave at `<destDir>/<name>`, or null when the move can't be undone for certain: a name that
 * is already in the destination (any case: the volume may ignore it), two sources with one name, or a source that is
 * the destination itself or already in it.
 */
export function planMoveUndo(sources: readonly string[], destDir: string, destNames: ReadonlySet<string>): MovedItem[] | null {
    const seen = new Set<string>();
    const items: MovedItem[] = [];
    for (const from of sources) {
        const name = baseName(from);
        const key = name.toLowerCase();
        if (destNames.has(key) || seen.has(key)) return null;
        const parent = parentOf(from);
        if (parent == null || samePath(parent, destDir) || samePath(from, destDir)) return null;
        seen.add(key);
        items.push({ from, to: joinPath(destDir, name) });
    }
    return items.length > 0 ? items : null;
}

/**
 * Whether a finished move landed as planned: the destination now holds, besides what it held before, exactly the names of
 * the items that went through. A conflict answered while the job ran (something created a name after the check, so the
 * item was kept under another one) adds a name more or fewer, and then Undo would move the wrong file. `after` is the
 * lower-case names now in the destination, or null when it couldn't be listed.
 */
export function landedAsPlanned(plan: UndoPlan, ok: readonly MovedItem[], after: ReadonlySet<string> | null): boolean {
    if (!after || ok.length === 0) return false;
    const expected = new Set(ok.map((i) => baseName(i.to).toLowerCase()));
    const added = [...after].filter((n) => !plan.before.has(n));
    return added.length === expected.size && added.every((n) => expected.has(n));
}

/** The items of a finished move that went through: those srv reports as failed are left out. */
export function movedOk(items: readonly MovedItem[], failedSources: readonly string[] | undefined): MovedItem[] {
    const failed = failedSources ?? [];
    return items.filter((i) => !failed.some((f) => samePath(f, i.from)));
}

/**
 * What can go back now: items still in the destination whose old name is free where they came from, grouped by the
 * folder they return to. `namesIn` is the lower-case names listed in a folder, or null when it couldn't be listed.
 */
export function restorable(
    items: readonly MovedItem[],
    namesIn: (dir: string) => ReadonlySet<string> | null
): { parent: string; paths: string[] }[] {
    const groups = new Map<string, { parent: string; paths: string[] }>();
    for (const item of items) {
        const parent = parentOf(item.from);
        const destParent = parentOf(item.to);
        if (parent == null || destParent == null) continue;
        const inDest = namesIn(destParent);
        const inParent = namesIn(parent);
        if (!inDest || !inParent) continue;
        const name = baseName(item.to).toLowerCase();
        if (!inDest.has(name) || inParent.has(name)) continue;
        // By the folder as spelled: on a volume that tells `A` from `a`, they are two folders.
        const group = groups.get(parent) ?? { parent, paths: [] };
        group.paths.push(item.to);
        groups.set(parent, group);
    }
    return [...groups.values()];
}
