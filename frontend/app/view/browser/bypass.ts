// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Bypass in an agent-driven browser pane: which sites a grant covers
// (SPEC_BROWSER_PANE_BYPASS_2026_10_16.md §4.2). srv decides; this is the
// same rule, so the header can show "paused" when the page leaves them.

/** The page's site (`https://example.com`), or undefined for a page with
 *  none (`about:blank`, `data:`). */
export function pageOrigin(url: string | undefined): string | undefined {
    if (!url) return undefined;
    try {
        const u = new URL(url);
        return u.protocol === "http:" || u.protocol === "https:" ? u.origin : undefined;
    } catch {
        return undefined;
    }
}

/** Is `url` on `origins`? Entries are in `allowed_origins` normal form:
 *  `https://example.com`, or `https://*.example.com` for the domain and its
 *  subdomains. A page with no site never is. */
export function originCovered(origins: string[], url: string | undefined): boolean {
    const target = pageOrigin(url);
    if (!target) return false;
    return origins.some((entry) => {
        if (entry === target) return true;
        const m = /^([a-z]+):\/\/\*\.(.+)$/.exec(entry);
        if (!m) return false;
        const [, scheme, rest] = m;
        if (!target.startsWith(`${scheme}://`)) return false;
        const host = target.slice(scheme.length + 3);
        return host === rest || host.endsWith(`.${rest}`);
    });
}

/** The durations the person can pick, in minutes (srv rounds anything else
 *  down to one of these). */
export const BYPASS_MINUTES = [15, 60, 240] as const;

export function minutesLabel(m: number): string {
    return m >= 60 ? `${m / 60} hour${m === 60 ? "" : "s"}` : `${m} min`;
}

/** A site for display: its host, without the scheme. */
export function siteLabel(origin: string): string {
    return origin.replace(/^https:\/\//, "");
}
