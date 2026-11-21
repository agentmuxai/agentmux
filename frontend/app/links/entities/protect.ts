// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Step 1 of the resolver (spec §4.1): the ranges of a text that are never touched. A fenced code block, a Markdown
 * image or link, an autolink, and an inline code span. A code span is protected too, with one exception (D4): a span
 * that is wholly one task number is a place to link, so it is reported apart for the task kind to read.
 */

import { RangeSet } from "./range-set";
import type { Range } from "./types";

export type Protected = { ranges: RangeSet<true>; codeSpans: Range[] };

const FENCE = /(^|\n)(```|~~~)[^\n]*\n[\s\S]*?(\n\2[^\n]*|$)/g;
const MD_LINK = /!?\[[^\]\n]*\]\([^)\n]*\)/g;
const AUTOLINK = /<(?:https?:\/\/|mailto:)[^>\s]*>/gi;
const CODE_SPAN = /(`+)(?!`)([^\n]*?[^`\n])\1(?!`)/g;

function collect(text: string, pattern: RegExp, into: Range[]): void {
    for (const m of text.matchAll(pattern)) {
        const start = (m.index ?? 0) + (pattern === FENCE ? m[1].length : 0);
        into.push({ start, end: (m.index ?? 0) + m[0].length });
    }
}

export function findProtected(text: string): Protected {
    const blocks: Range[] = [];
    collect(text, FENCE, blocks);
    collect(text, MD_LINK, blocks);
    collect(text, AUTOLINK, blocks);
    const blockSet = RangeSet.union(blocks);
    const spans: Range[] = [];
    collect(text, CODE_SPAN, spans);
    // A span inside a fenced block or a link is already protected; the rest are protected and reported.
    const codeSpans = spans.filter((span) => blockSet.find(span) === undefined);
    return { ranges: RangeSet.union([...blocks, ...codeSpans]), codeSpans };
}
