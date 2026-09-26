// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The count a Grep row's pill shows, read from Claude Code's result text.
 * Each `output_mode` has its own shape (captured from live Grep calls):
 *
 *  - files_with_matches (the default): "Found N files\n<path>…", or
 *    "No files found".
 *  - content: one "path:line:text" line per match, "--" between context
 *    groups, and an optional "[Showing results with pagination …]" notice;
 *    "No matches found" when empty.
 *  - count: "path:N" lines and a "Found T total occurrences across M files."
 *    trailer.
 *
 * Counting every non-empty line (the first version) read 3 files as
 * "4 matches" and an empty search as "1 match" (Opaz P1 on #3877).
 */

export interface GrepCount {
    n: number;
    noun: "file" | "match";
}

const FILES_HEADER = /^Found (\d+) files?\b/;
const COUNT_TRAILER = /^Found (\d+) total occurrences? across \d+ files?\.?$/m;

export function grepResultCount(text: string): GrepCount {
    const trimmed = text.trim();
    const total = COUNT_TRAILER.exec(trimmed);
    if (total) return { n: Number(total[1]), noun: "match" };
    if (trimmed.startsWith("No files found")) return { n: 0, noun: "file" };
    if (trimmed.startsWith("No matches found")) return { n: 0, noun: "match" };
    const files = FILES_HEADER.exec(trimmed);
    if (files) return { n: Number(files[1]), noun: "file" };
    const n = trimmed
        .split("\n")
        .filter((l) => l.trim() && l.trim() !== "--" && !l.startsWith("[Showing results")).length;
    return { n, noun: "match" };
}
