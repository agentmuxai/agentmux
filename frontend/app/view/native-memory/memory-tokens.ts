// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How the Memory pane writes sizes: tokens first, always labelled as an
 * estimate; bytes and lines only in the tooltip
 * (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §3, §5.1). The counts come from srv
 * (`tokens_est`), estimated the way `estimateTokenCount` does.
 */

import { formatCompactNumber, formatExactNumber } from "@/util/format-count";

/** How every count here was made, for tooltips. */
export const TOKEN_ESTIMATE_NOTE = "Estimated: characters ÷ 4";

/** Byte-count label ("4.8 KB"), base 1024. */
export function formatFileSize(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes < 0) return "";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** The chip: "~1.2k tok". */
export function tokensLabel(tokens: number): string {
    return `~${formatCompactNumber(Math.max(0, Math.round(tokens)))} tok`;
}

/** The chip's tooltip: "~1,234 tokens (est.) · 4.8 KB · 86 lines", then how
 *  it was estimated. Segments with no value are left out. */
export function tokensTitle(tokens: number, sizeBytes?: number, lines?: number): string {
    const n = Math.max(0, Math.round(tokens));
    const parts = [`~${formatExactNumber(n)} ${n === 1 ? "token" : "tokens"} (est.)`];
    if (sizeBytes != null) parts.push(formatFileSize(sizeBytes));
    if (lines) parts.push(lines === 1 ? "1 line" : `${formatExactNumber(lines)} lines`);
    return `${parts.join(" · ")}\n${TOKEN_ESTIMATE_NOTE}`;
}

/** `part` as a share of `total`, "17%"; "<1%" for a share that rounds to
 *  nothing; "" when there is no total to share. */
export function shareLabel(part: number, total: number): string {
    if (!(total > 0) || !(part > 0)) return "";
    const pct = Math.round((part / total) * 100);
    return pct === 0 ? "<1%" : `${pct}%`;
}

/** What Claude Code loads of an agent's `MEMORY.md`: its first 200 lines,
 *  and no more than 25 KB (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.3). */
export const CLAUDE_MEMORY_INDEX_LIMIT = { lines: 200, bytes: 25 * 1024 } as const;

/** What Claude Code leaves out of an index file this size, or "" when it
 *  loads all of it. */
export function memoryIndexOverLimit(file: { lines: number; size_bytes: number }): string {
    const { lines, bytes } = CLAUDE_MEMORY_INDEX_LIMIT;
    const over: string[] = [];
    if (file.lines === lines + 1) over.push(`line ${file.lines} isn't loaded`);
    else if (file.lines > lines) over.push(`lines ${lines + 1}–${file.lines} aren't loaded`);
    if (file.size_bytes > bytes) over.push(`at ${formatFileSize(file.size_bytes)} it's past the 25 KB loaded`);
    return over.join("; ");
}
