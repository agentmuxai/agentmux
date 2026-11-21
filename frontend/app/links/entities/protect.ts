// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Step 1 of the resolver (spec §4.1): the ranges of a text that are never touched. A fenced code block, a Markdown
 * image or link, an autolink, and an inline code span. A code span is protected too, with one exception (D4): a span
 * that is wholly one task number is a place to link, so it is reported apart for the task kind to read.
 */

import type { Range } from "./types";

export type Protected = { ranges: Range[]; codeSpans: Range[] };

const FENCE = /(^|\n)(```|~~~)[^\n]*\n[\s\S]*?(\n\2[^\n]*|$)/g;
const MD_LINK = /!?\[[^\]\n]*\]\([^)\n]*\)/g;
const AUTOLINK = /<(?:https?:\/\/|mailto:)[^>\s]*>/gi;
const CODE_SPAN = /(`+)(?!`)([^\n]*?[^`\n])\1(?!`)/g;

function collect(text: string, pattern: RegExp, into: Range[]): void {
    pattern.lastIndex = 0;
    for (const m of text.matchAll(pattern)) {
        const start = (m.index ?? 0) + (pattern === FENCE ? m[1].length : 0);
        into.push({ start, end: (m.index ?? 0) + m[0].length });
    }
}

export function overlaps(a: Range, b: Range): boolean {
    return a.start < b.end && b.start < a.end;
}

export function findProtected(text: string): Protected {
    const ranges: Range[] = [];
    collect(text, FENCE, ranges);
    collect(text, MD_LINK, ranges);
    collect(text, AUTOLINK, ranges);
    const codeSpans: Range[] = [];
    collect(text, CODE_SPAN, codeSpans);
    // A span inside a fenced block or a link is already protected; the rest are protected and reported.
    const free = codeSpans.filter((span) => !ranges.some((r) => overlaps(r, span)));
    ranges.push(...free);
    return { ranges, codeSpans: free };
}

export function insideAny(range: Range, ranges: Range[]): boolean {
    return ranges.some((r) => overlaps(r, range));
}
