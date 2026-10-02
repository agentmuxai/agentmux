// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sorting a folder's entries: folders first, then by the chosen column, with
 * names compared naturally (`file2` before `file10`) and case-insensitively.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.2.
 */

import type { FsEntry } from "@/types/rpc/FsEntry";

export type SortKey = "name" | "modified" | "size" | "kind";
export type SortDir = "asc" | "desc";

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/** The extension a Kind column sorts and shows by: lower case, no dot. */
export function extensionOf(name: string): string {
    const dot = name.lastIndexOf(".");
    return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

function compareBy(key: SortKey, a: FsEntry, b: FsEntry): number {
    switch (key) {
        case "modified":
            return (a.mtime ?? 0) - (b.mtime ?? 0);
        case "size":
            return (a.size ?? -1) - (b.size ?? -1);
        case "kind":
            return collator.compare(extensionOf(a.name), extensionOf(b.name));
        default:
            return 0;
    }
}

/**
 * A sorted copy. Folders always come first, whichever way the column runs;
 * ties fall back to the name, ascending, so the order is stable and
 * predictable.
 */
export function sortEntries(entries: readonly FsEntry[], key: SortKey, dir: SortDir): FsEntry[] {
    const sign = dir === "asc" ? 1 : -1;
    return [...entries].sort((a, b) => {
        if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
        const byKey = key === "name" ? collator.compare(a.name, b.name) : compareBy(key, a, b);
        if (byKey !== 0) return sign * byKey;
        return collator.compare(a.name, b.name);
    });
}
