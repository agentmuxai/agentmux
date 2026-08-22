// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Pure bookmark list transforms, extracted out of browser-nav-bar.tsx so
// the toggle/dedupe logic is directly unit-testable without rendering the
// Solid component. See
// docs/specs/SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md.

/** Exact-URL lookup — bookmarks are deduped by URL, not by id. */
export function findBookmark(list: BrowserBookmark[], url: string): BrowserBookmark | undefined {
    return list.find((b) => b.url === url);
}

export interface ToggleBookmarkInput {
    url: string;
    title: string;
    faviconUrl: string;
    /** Injected so this stays pure/testable — no direct crypto.randomUUID() call. */
    newId: () => string;
    /** Injected so this stays pure/testable — no direct Date.now() call. */
    now: () => number;
}

/**
 * Add-or-remove the given URL from the list, keyed by exact URL match so
 * repeated toggling of the same page never produces duplicate entries
 * (append-only would). Falls back to the URL itself as the title when no
 * page title is available yet (e.g. toggled before the title-change event
 * has landed).
 */
export function toggleBookmark(list: BrowserBookmark[], input: ToggleBookmarkInput): BrowserBookmark[] {
    const existing = findBookmark(list, input.url);
    if (existing) {
        return list.filter((b) => b.id !== existing.id);
    }
    return [
        ...list,
        {
            id: input.newId(),
            title: input.title || input.url,
            url: input.url,
            favicon_url: input.faviconUrl,
            created_at: input.now(),
        },
    ];
}
