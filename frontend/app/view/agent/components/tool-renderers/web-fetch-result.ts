// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tolerant extraction of WebFetch tool results into a display-ready shape.
 * Handles both the plain-string form (most common) and the structured-object
 * form ({ url, status, content, title, truncated }).
 *
 * See SPEC_WEBFETCH_CONTENT_VIEW_2026_06_22.md.
 */

export interface FetchResultData {
    url?: string;
    title?: string;
    status?: number;
    content: string;
    truncated?: boolean;
    contentType?: string;
}

function str(v: unknown): string | null {
    return typeof v === "string" && v.trim().length > 0 ? v.trim() : null;
}

function num(v: unknown): number | null {
    return typeof v === "number" && isFinite(v) ? v : null;
}

function bool(v: unknown): boolean | undefined {
    return typeof v === "boolean" ? v : undefined;
}

/**
 * Extract a FetchResultData from an arbitrary tool result, or null if the
 * result is not fetch-shaped. Returns non-null for any string (content only)
 * or any object that has a recognizable content field.
 */
export function extractFetchResult(result: unknown): FetchResultData | null {
    // Shape A: plain string
    if (typeof result === "string" && result.trim().length > 0) {
        return { content: result.trim() };
    }

    // Shape B: structured object
    if (result && typeof result === "object" && !Array.isArray(result)) {
        const o = result as Record<string, unknown>;

        // content is required
        const content =
            str(o.content) ??
            str(o.body) ??
            str(o.text) ??
            str(o.html) ??
            str(o.data);
        if (!content) return null;

        const url = str(o.url) ?? str(o.uri) ?? str(o.href) ?? undefined;
        const title = str(o.title) ?? str(o.page_title) ?? undefined;
        const status = num(o.status) ?? num(o.status_code) ?? num(o.statusCode) ?? undefined;
        const truncated = bool(o.truncated) ?? bool(o.is_truncated);
        const contentType =
            str(o.contentType) ??
            str(o.content_type) ??
            str(o.mimeType) ??
            str(o.mime_type) ??
            undefined;

        return { url, title, status: status ?? undefined, content, truncated, contentType };
    }

    return null;
}

/** True when the content looks like JSON (try-parse prefix). */
export function looksLikeJson(content: string): boolean {
    const trimmed = content.trimStart();
    if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) return false;
    try {
        JSON.parse(trimmed.length > 2000 ? trimmed.slice(0, 2000) + "}" : trimmed);
        return true;
    } catch {
        // heuristic: still likely JSON if it starts with { or [
        return trimmed.startsWith("{") || trimmed.startsWith("[");
    }
}

/** Map HTTP status code to a short label. */
export function httpStatusText(status: number): string {
    const map: Record<number, string> = {
        200: "OK", 201: "Created", 204: "No Content",
        301: "Moved", 302: "Found", 304: "Not Modified",
        400: "Bad Request", 401: "Unauthorized", 403: "Forbidden",
        404: "Not Found", 405: "Not Allowed", 429: "Rate Limited",
        500: "Server Error", 502: "Bad Gateway", 503: "Unavailable",
    };
    return map[status] ?? String(status);
}

/** CSS class suffix for a status code. */
export function statusClass(status: number): "ok" | "redirect" | "error" {
    if (status >= 200 && status < 300) return "ok";
    if (status >= 300 && status < 400) return "redirect";
    return "error";
}
