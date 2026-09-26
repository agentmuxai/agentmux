// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * URL helpers shared by the web tool renderers (SearchResults, WebFetchResult),
 * which each used to carry an identical copy.
 */

import { getApi } from "@/store/global";

/** Strip protocol; show host + path (e.g. "example.com/page"). */
export function prettyUrl(url: string): string {
    try {
        const u = new URL(url);
        const path = u.pathname === "/" ? "" : u.pathname.replace(/\/$/, "");
        return `${u.host}${path}`;
    } catch {
        return url;
    }
}

/** Just the hostname, or "" for an unparseable URL. */
export function hostname(url: string): string {
    try {
        return new URL(url).hostname;
    } catch {
        return "";
    }
}

/** 16 px favicon for a host, via Google's favicon service. Callers hide the
 *  <img> on error. */
export function faviconSrc(host: string): string {
    return `https://www.google.com/s2/favicons?domain=${host}&sz=16`;
}

/** Open in the system browser; best-effort. */
export function openUrl(url: string): void {
    try {
        getApi().openExternal(url);
    } catch {
        /* best-effort */
    }
}
