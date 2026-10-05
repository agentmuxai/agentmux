// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shiki colours for an embedded heredoc body, as plain data: runs of text
 * with CSS variables, never HTML. The caller renders them as spans with text
 * children, so nothing in a command can become markup.
 *
 * Both themes are highlighted at once: each run carries `--shiki-dark` and
 * `--shiki-light`, and the stylesheet picks one by `[data-theme-polarity]`,
 * so a theme flip needs no re-highlight.
 *
 * The body keeps its plain rendering until this resolves; if it fails, is
 * too big, or Shiki's text doesn't match the body exactly, it stays plain
 * (spec §3.3, §4).
 */

/** A run of body text and the Shiki CSS variables that colour it. */
export interface ColoredRun {
    text: string;
    style?: Record<string, string>;
}

const THEMES = { dark: "github-dark-high-contrast", light: "github-light-high-contrast" } as const;

// The same caps as HighlightedCode.
const CAP_BYTES = 200 * 1024;
const CAP_LINES = 2000;
const CACHE_LIMIT = 100;

const cache = new Map<string, ColoredRun[]>();

let shikiModule: typeof import("../shiki-highlighter") | null = null;
const getShiki = async () => (shikiModule ??= await import("../shiki-highlighter"));

function key(text: string, lang: string): string {
    return `${lang}\0${text}`;
}

/** Already-highlighted runs, or undefined. Refreshes recency. */
export function cachedRuns(text: string, lang: string): ColoredRun[] | undefined {
    const k = key(text, lang);
    const hit = cache.get(k);
    if (hit) {
        cache.delete(k);
        cache.set(k, hit);
    }
    return hit;
}

function remember(k: string, runs: ColoredRun[]): void {
    cache.set(k, runs);
    if (cache.size > CACHE_LIMIT) {
        const oldest = cache.keys().next().value;
        if (oldest !== undefined) cache.delete(oldest);
    }
}

/** Only Shiki's own variables reach the DOM. */
function shikiVars(style: Record<string, string> | undefined): Record<string, string> | undefined {
    if (!style) return undefined;
    const out = Object.fromEntries(Object.entries(style).filter(([k]) => k.startsWith("--shiki-")));
    return Object.keys(out).length ? out : undefined;
}

/** Highlight `text` as `lang`; null when it can't be done faithfully. */
export async function highlightBody(text: string, lang: string): Promise<ColoredRun[] | null> {
    const k = key(text, lang);
    const hit = cache.get(k);
    if (hit) return hit;
    if (text.length > CAP_BYTES || text.split("\n").length > CAP_LINES) return null;
    try {
        const { codeToTokens } = await getShiki();
        const { tokens } = await codeToTokens(text, { lang: lang as never, themes: THEMES, defaultColor: false });
        const runs: ColoredRun[] = [];
        tokens.forEach((line, i) => {
            if (i > 0) runs.push({ text: "\n" });
            for (const t of line) {
                const style = shikiVars(t.htmlStyle as Record<string, string> | undefined);
                runs.push(style ? { text: t.content, style } : { text: t.content });
            }
        });
        // Never show text that differs from the command (spec §4): Shiki
        // normalises line endings, so a body with \r\n stays plain.
        if (runs.map((r) => r.text).join("") !== text) return null;
        remember(k, runs);
        return runs;
    } catch {
        return null;
    }
}
