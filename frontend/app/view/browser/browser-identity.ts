// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which identity (cookie jar) a browser tab browses as, and opening a new tab
 * in one (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §5).
 *
 * A tab's identity is its block's `browser:identity` meta, fixed when the tab
 * is opened: absent for Personal (the shared jar every tab had before this),
 * `incognito:<jar>` for an in-memory jar of its own, `profile:<id>` for a
 * named profile (Phase 2). A popup a tab opens carries the same value, so it
 * shares the tab's jar.
 */

export const IDENTITY_META_KEY = "browser:identity";
export const INCOGNITO_ICON = "user-secret";

export type BrowserIdentity =
    | { kind: "personal" }
    | { kind: "incognito"; jar: string }
    | { kind: "profile"; id: string };

export function parseIdentity(value: unknown): BrowserIdentity {
    if (typeof value === "string") {
        const incognito = /^incognito:([A-Za-z0-9-]{8,64})$/.exec(value);
        if (incognito) return { kind: "incognito", jar: incognito[1] };
        const profile = /^profile:([a-z0-9-]{1,64})$/.exec(value);
        if (profile) return { kind: "profile", id: profile[1] };
    }
    return { kind: "personal" };
}

/** A fresh Incognito identity: a jar of its own, shared with nothing else. */
export function newIncognitoIdentity(): string {
    return `incognito:${crypto.randomUUID()}`;
}

/** What a new tab opens: the current page, so "this site as another identity"
 *  is one click; Home when the current page isn't a web page. */
export function newTabUrl(currentUrl: string | undefined, home: string): string {
    return currentUrl && /^https?:\/\//i.test(currentUrl) ? currentUrl : home;
}

/** Whether this platform can give a pane a jar of its own yet. Windows only
 *  until the Linux/macOS work (spec §7.2, Phase 3). */
export function canOpenIncognito(platform: string): boolean {
    return platform === "win32";
}
