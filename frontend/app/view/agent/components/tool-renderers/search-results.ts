// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tolerant extraction of web-search-style results from a tool result, so
 * `WebSearch` (and look-alike tools) can render as result cards instead of a
 * JSON blob. Defensive by design: provider result shapes vary, so this accepts
 * several common shapes and returns `null` for anything it doesn't recognize —
 * the renderer then falls back to today's JSON view (no regression).
 *
 * See SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md §5.4.
 */

export interface SearchResultItem {
    title: string;
    url: string;
    snippet?: string;
    date?: string;
    index?: number;
}

/** Keys whose array value may hold the result list. */
const ARRAY_KEYS = ["results", "content", "items", "links", "data", "web_search_results"] as const;

function str(v: unknown): string | null {
    return typeof v === "string" && v.trim().length > 0 ? v.trim() : null;
}

function tryParseJsonArray(v: unknown): unknown[] | null {
    if (typeof v !== "string") return null;
    try {
        const p = JSON.parse(v);
        return Array.isArray(p) ? p : null;
    } catch {
        return null;
    }
}

/** Locate the array of result objects (top-level array, or under a known key). */
function findResultArray(result: unknown): unknown[] | null {
    if (Array.isArray(result)) return result;
    // Top-level JSON-encoded array string
    const topLevel = tryParseJsonArray(result);
    if (topLevel) return topLevel;
    if (result && typeof result === "object") {
        const o = result as Record<string, unknown>;
        for (const k of ARRAY_KEYS) {
            if (Array.isArray(o[k])) return o[k] as unknown[];
            // Value may be a JSON-encoded array (e.g. when buildToolResults wraps
            // block.content as { content: "[{...}]" } for string-body tool results).
            const parsed = tryParseJsonArray(o[k] as unknown);
            if (parsed) return parsed;
        }
    }
    return null;
}

/**
 * Extract search-result cards, or `null` if the result isn't search-shaped. A
 * result item must carry a URL; title/snippet are best-effort across common
 * field names.
 */
export function extractSearchResults(result: unknown): SearchResultItem[] | null {
    const arr = findResultArray(result);
    if (!arr) return null;
    const items: SearchResultItem[] = [];
    for (let i = 0; i < arr.length; i++) {
        const el = arr[i];
        if (!el || typeof el !== "object") continue;
        const o = el as Record<string, unknown>;
        const url = str(o.url) ?? str(o.link) ?? str(o.uri);
        if (!url) continue; // a search result must have a URL
        const title = str(o.title) ?? str(o.name) ?? str(o.heading) ?? url;
        // page_age is a date string ("June 15, 2026"), not a snippet — keep it separate
        const snippet =
            str(o.snippet) ??
            str(o.description) ??
            str(o.text) ??
            (typeof o.content === "string" ? str(o.content) : undefined) ??
            undefined;
        const date = str(o.page_age) ?? str(o.published_date) ?? str(o.date) ?? undefined;
        items.push({ title, url, snippet, date, index: i + 1 });
    }
    return items.length > 0 ? items : null;
}

/** Claude Code's WebSearch result, parsed. */
export interface ParsedWebSearch {
    query?: string;
    /** title + url + index only: Claude Code sends no snippet or date. */
    links: SearchResultItem[];
    /** The search sub-model's markdown answer, REMINDER footer removed. */
    summary?: string;
}

const QUERY_LINE = /^Web search results for query: "(.*)"\s*$/;
const LINKS_PREFIX = "Links: ";
/** The footer Claude Code appends for the model, not for the user. */
const REMINDER = /\n*REMINDER: You MUST include the sources above[\s\S]*$/;

/**
 * Parse Claude Code's WebSearch `tool_result.content`: one prose string with a
 * `Web search results for query: "…"` line, one `Links: [...]` line of inline
 * JSON per search round, a markdown summary, and a REMINDER footer. Returns
 * null unless it yields at least one link or a non-empty summary.
 * SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.3.
 */
export function parseWebSearchText(text: string): ParsedWebSearch | null {
    const lines = text.replace(REMINDER, "").split("\n");
    const header = QUERY_LINE.exec(lines[0] ?? "");
    if (!header) return null;
    const links: SearchResultItem[] = [];
    const seen = new Set<string>();
    const rest: string[] = [];
    for (const line of lines.slice(1)) {
        if (!line.startsWith(LINKS_PREFIX)) {
            rest.push(line);
            continue;
        }
        const arr = tryParseJsonArray(line.slice(LINKS_PREFIX.length));
        for (const el of arr ?? []) {
            const o = (el ?? {}) as Record<string, unknown>;
            const url = str(o.url);
            if (!url || seen.has(url)) continue;
            seen.add(url);
            links.push({ title: str(o.title) ?? url, url, index: links.length + 1 });
        }
    }
    const summary = rest.join("\n").trim() || undefined;
    if (links.length === 0 && !summary) return null;
    return { query: header[1], links, summary };
}

/** `parseWebSearchText` over a raw string or the translator's `{content}`
 *  wrapper; null for anything else (array shapes go through
 *  `extractSearchResults`). */
export function extractWebSearch(result: unknown): ParsedWebSearch | null {
    const text =
        typeof result === "string"
            ? result
            : result && typeof result === "object" && typeof (result as { content?: unknown }).content === "string"
              ? ((result as { content: string }).content)
              : null;
    return text == null ? null : parseWebSearchText(text);
}

export interface SourceGroup {
    /** Hostname without a leading "www.". */
    domain: string;
    links: SearchResultItem[];
}

/** Group links by domain, in first-seen order — one sources-strip chip each. */
export function groupLinksByDomain(links: readonly SearchResultItem[]): SourceGroup[] {
    const groups = new Map<string, SourceGroup>();
    for (const link of links) {
        let domain: string;
        try {
            domain = new URL(link.url).hostname.replace(/^www\./, "");
        } catch {
            domain = link.url;
        }
        const g = groups.get(domain);
        if (g) g.links.push(link);
        else groups.set(domain, { domain, links: [link] });
    }
    return [...groups.values()];
}

/** True when the result looks like a list of web-search results. */
export function looksLikeSearchResults(result: unknown): boolean {
    return extractSearchResults(result) != null;
}
