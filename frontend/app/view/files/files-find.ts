// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Finding by name in the folder shown and every folder under it (docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md
 * section 12, phase 3, "fuzzy find in the folder tree"). Two pure parts and one that takes its listings as a
 * function: a fuzzy score of a path against what was typed, a ranking of an index of paths, and a bounded walk that
 * builds that index. A found entry is named by its path from the shown folder (`src/lib/a.ts`, always with `/`),
 * as in the tree view, so selection, rename and the operations treat it like any row.
 */

import type { FsEntry } from "@/types/rpc/FsEntry";

const SEPARATORS = new Set(["/", ".", "_", "-", " "]);

/** The last segment of a path from the shown folder. */
function nameOf(rel: string): string {
    return rel.slice(rel.lastIndexOf("/") + 1);
}

/** The folders a path is in, "" at the first level. */
export function whereOf(rel: string): string {
    const i = rel.lastIndexOf("/");
    return i < 0 ? "" : rel.slice(0, i);
}

/**
 * How well `query` matches `rel`, or null when it doesn't: the query's letters must appear in order in the path,
 * case aside. A higher score is a better match: the whole query inside the file's own name beats letters spread over
 * folders, a match at the start of the name or of a word (after a separator, or a capital) beats one in the middle,
 * letters that follow each other beat ones that don't, and a shorter path wins a tie.
 */
export function fuzzyScore(query: string, rel: string): number | null {
    const q = query.trim().toLowerCase();
    if (q === "") return null;
    const text = rel.toLowerCase();
    const nameStart = rel.lastIndexOf("/") + 1;
    let score = 0;
    let at = 0;
    let previous = -2;
    for (const ch of q) {
        const found = text.indexOf(ch, at);
        if (found < 0) return null;
        const boundary = found === 0 || SEPARATORS.has(rel[found - 1]) || (rel[found] !== rel[found].toLowerCase() && rel[found - 1] === rel[found - 1].toLowerCase());
        score += 1;
        if (found === previous + 1) score += 5;
        if (boundary) score += 4;
        if (found === nameStart) score += 6;
        // A letter far from the one before it costs a little, and a letter in a folder's name costs more than one in the file's.
        score -= Math.min(3, found - at);
        if (found < nameStart) score -= 1;
        previous = found;
        at = found + 1;
    }
    const inName = text.slice(nameStart).indexOf(q);
    if (inName >= 0) score += 30 + (inName === 0 ? 10 : 0);
    return score * 100 - rel.length;
}

/** The best `limit` of `index` for `query`, best first (ties: shorter path, then alphabetical). */
export function rankMatches(index: readonly FsEntry[], query: string, limit: number): FsEntry[] {
    const scored: { e: FsEntry; s: number }[] = [];
    for (const e of index) {
        const s = fuzzyScore(query, e.name);
        if (s !== null) scored.push({ e, s });
    }
    scored.sort((a, b) => b.s - a.s || a.e.name.length - b.e.name.length || (a.e.name < b.e.name ? -1 : a.e.name > b.e.name ? 1 : 0));
    return scored.slice(0, limit).map((x) => x.e);
}

/** Folders never searched: a version control folder and a dependency folder hold what nobody looks for by name. */
const SKIPPED_FOLDERS = new Set([".git", "node_modules"]);

export interface CrawlLimits {
    /** The most folders listed. */
    maxFolders: number;
    /** The most entries kept. */
    maxEntries: number;
    /** Wall time, in ms. */
    maxMs: number;
    /** Listings in flight at once. */
    parallel: number;
}

export const DEFAULT_CRAWL: CrawlLimits = { maxFolders: 1500, maxEntries: 60_000, maxMs: 6000, parallel: 4 };

export type CrawlEnd = "done" | "limit" | "aborted";

/**
 * Walks the folders under the shown one, breadth first, `parallel` listings at a time, and hands each folder's
 * entries (named by their path from the shown folder) to `onBatch` as they come. It stops at a limit or when
 * `aborted()` says so. `list(rel)` returns a folder's entries, or null when it can't be read (skipped). A link to a
 * folder is a row but is not walked into, so a link back up can't make it loop; neither are the skipped folders,
 * and a hidden folder only when `hidden` says so.
 */
export async function crawlFolders(
    list: (rel: string) => Promise<readonly FsEntry[] | null>,
    onBatch: (entries: FsEntry[]) => void,
    aborted: () => boolean,
    hidden: boolean,
    limits: CrawlLimits = DEFAULT_CRAWL,
    now: () => number = Date.now
): Promise<{ end: CrawlEnd; folders: number; entries: number }> {
    const queue: string[] = [""];
    const startedAt = now();
    let folders = 0;
    let entries = 0;
    let end: CrawlEnd = "done";
    const over = (): boolean => {
        if (aborted()) {
            end = "aborted";
            return true;
        }
        if (folders >= limits.maxFolders || entries >= limits.maxEntries || now() - startedAt > limits.maxMs) {
            end = "limit";
            return true;
        }
        return false;
    };
    // A worker takes the next folder when its listing ends. One that finds the queue empty waits while another is
    // still listing, since that listing may add folders; it stops once nobody is listing.
    let active = 0;
    const worker = async (): Promise<void> => {
        for (;;) {
            if (over()) return;
            const rel = queue.shift();
            if (rel === undefined) {
                if (active === 0) return;
                await new Promise((resolve) => setTimeout(resolve, 1));
                continue;
            }
            folders++;
            active++;
            try {
                const got = await list(rel);
                if (aborted()) {
                    end = "aborted";
                    return;
                }
                if (!got) continue;
                const batch: FsEntry[] = [];
                for (const e of got) {
                    if (e.hidden && !hidden) continue;
                    const path = rel === "" ? e.name : `${rel}/${e.name}`;
                    batch.push({ ...e, name: path });
                    if (e.is_dir && !e.is_symlink && !SKIPPED_FOLDERS.has(e.name)) queue.push(path);
                }
                entries += batch.length;
                if (batch.length > 0) onBatch(batch);
            } finally {
                active--;
            }
        }
    };
    await Promise.all(Array.from({ length: limits.parallel }, worker));
    return { end, folders, entries };
}
