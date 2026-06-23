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

/** True when the result looks like a list of web-search results. */
export function looksLikeSearchResults(result: unknown): boolean {
    return extractSearchResults(result) != null;
}
