// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Syntax highlighting for previews, as tokens per line
 * (REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §5.1). One Shiki loader and
 * one cache for every preview, instead of an HTML string per component pushed
 * in with `innerHTML`.
 *
 * Tokens come back per input line, so a preview keeps its one-block-per-line
 * DOM; a diff's two sides are tokenized separately so grammar state can't run
 * from a deleted line into an added one.
 */

import type { PreviewDoc } from "./types";

export const SHIKI_THEME = "github-dark-high-contrast";

/** One highlighted run. `fontStyle` is Shiki's bit set: 1 italic, 2 bold, 4 underline. */
export interface CodeToken {
    content: string;
    color?: string;
    fontStyle?: number;
}

/** Don't highlight more than this much text at once: it blocks the main thread. */
const MAX_HIGHLIGHT_CHARS = 200 * 1024;
const CACHE_ENTRIES = 200;

let shiki: Promise<typeof import("../components/shiki-highlighter")> | null = null;
/** The Shiki module, loaded once on first use. */
export const loadShiki = () => (shiki ??= import("../components/shiki-highlighter"));

const cache = new Map<string, CodeToken[][]>();

/** Tokens for each line of `code`, or null for plain text, too much text, or a
 *  grammar Shiki can't load. */
export async function tokenize(code: string, lang: string | undefined): Promise<CodeToken[][] | null> {
    if (!code || !lang || lang === "text" || code.length > MAX_HIGHLIGHT_CHARS) return null;
    const key = `${lang}\u0000${code}`;
    const hit = cache.get(key);
    if (hit) return hit;
    try {
        const { codeToTokens } = await loadShiki();
        const { tokens } = await codeToTokens(code, { lang: lang as never, theme: SHIKI_THEME });
        const lines = tokens.map((line) =>
            line.map((t) => ({ content: t.content, color: t.color, fontStyle: t.fontStyle }))
        );
        if (cache.size >= CACHE_ENTRIES) cache.delete(cache.keys().next().value!);
        cache.set(key, lines);
        return lines;
    } catch (e) {
        console.warn(`[preview] no highlighting for ${lang}`, e);
        return null;
    }
}

/**
 * Tokens for each line of a code or diff preview, aligned with `doc.lines`
 * (null where a line isn't highlighted, such as a diff's `@@` header), or null
 * when the preview isn't highlighted at all.
 */
export async function highlightDoc(doc: PreviewDoc): Promise<(CodeToken[] | null)[] | null> {
    if (!doc.lang || doc.lang === "text") return null;
    if (doc.kind === "code") return tokenize(doc.lines.map((l) => l.text).join("\n"), doc.lang);
    if (doc.kind !== "diff") return null;
    const oldSide = doc.lines.filter((l) => l.marker === " " || l.marker === "-");
    const newSide = doc.lines.filter((l) => l.marker === " " || l.marker === "+");
    const [oldTokens, newTokens] = await Promise.all([
        tokenize(oldSide.map((l) => l.text).join("\n"), doc.lang),
        tokenize(newSide.map((l) => l.text).join("\n"), doc.lang),
    ]);
    if (!oldTokens && !newTokens) return null;
    let oi = 0;
    let ni = 0;
    return doc.lines.map((l) => {
        if (l.marker === "-") return oldTokens?.[oi++] ?? null;
        if (l.marker === "+") return newTokens?.[ni++] ?? null;
        if (l.marker === " ") {
            oi++;
            return newTokens?.[ni++] ?? null;
        }
        return null;
    });
}
