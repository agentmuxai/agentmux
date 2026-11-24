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
        const key = parent.toLowerCase();
        const group = groups.get(key) ?? { parent, paths: [] };
        group.paths.push(item.to);
        groups.set(key, group);
    }
    return [...groups.values()];
}
