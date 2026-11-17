// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
// A browser tab under a non-default identity keeps the site's icon and carries a small mark on it
// (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §4).

import { describe, expect, it, vi } from "vitest";

vi.mock("./browser-profiles", () => ({
    profileById: (id: string) =>
        ({
            work: { id: "work", name: "Work", color: "#3b82f6" },
            plain: { id: "plain", name: "Plain" },
            night: { id: "night", name: "Night", color: "#000000" },
            paper: { id: "paper", name: "Paper", color: "#ffffff" },
        })[id],
}));

import type { PaneTabContext } from "@/app/element/pane-tab-model";
import { browserPaneTab } from "./browser";
import { parseIdentity } from "@/app/util/browser-identity";
import { paneRoleColor } from "@/app/block/pane-color-scheme";
import { identityBadge, profileMarkColor } from "./browser-tab-badge";

const ctx = (meta: Record<string, unknown>): PaneTabContext => ({ blockId: "b1", view: "browser", meta: { view: "browser", ...meta } as never, ordinal: 1, liveViewModel: null });
const tab = browserPaneTab.tab!;

describe("identityBadge", () => {
    it("has none for Personal", () => {
        expect(identityBadge(parseIdentity(undefined))).toBeUndefined();
    });

    it("is the named profile's colour, labelled with its name", () => {
        expect(identityBadge(parseIdentity("profile:work"))).toEqual({ color: "#3b82f6", label: "Work" });
    });

    it("draws a Black or White profile's mark in the neutral's line tone, which shows on a tab of its own colour", () => {
        expect(identityBadge(parseIdentity("profile:night"))).toEqual({ color: paneRoleColor(undefined, "black", false, "identity"), label: "Night" });
        expect(identityBadge(parseIdentity("profile:paper"))).toEqual({ color: paneRoleColor(undefined, "white", false, "identity"), label: "Paper" });
        expect(identityBadge(parseIdentity("profile:night"))?.color).not.toBe("#000000");
    });

    it("falls back to the accent for a profile with no colour, and for one that no longer exists", () => {
        expect(identityBadge(parseIdentity("profile:plain"))).toEqual({ color: "var(--accent-color)", label: "Plain" });
        expect(identityBadge(parseIdentity("profile:gone"))).toEqual({ color: "var(--accent-color)", label: "Browser profile" });
    });

    it("is a muted mark for Incognito", () => {
        expect(identityBadge(parseIdentity("incognito:11111111-2222-3333-4444-555555555555"))).toEqual({ color: "var(--secondary-text-color)", label: "Incognito" });
    });
});

describe("profileMarkColor", () => {
    it("is a hue profile's own colour, and nothing for a profile with none", () => {
        expect(profileMarkColor("#3b82f6")).toBe("#3b82f6");
        expect(profileMarkColor(undefined)).toBeUndefined();
    });

    it("is the neutral's line tone for Black and White, which shows on chrome of the neutral's own colour", () => {
        expect(profileMarkColor("#000000")).toBe(paneRoleColor(undefined, "black", false, "identity"));
        expect(profileMarkColor("#FFFFFF")).toBe(paneRoleColor(undefined, "white", false, "identity"));
        expect(profileMarkColor("#000000")).not.toBe("#000000");
    });
});

describe("the browser's tab", () => {
    it("never replaces the site's icon with a profile's or Incognito's", () => {
        expect(tab.icon!(ctx({ "browser:identity": "profile:work" }))).toBeUndefined();
        expect(tab.icon!(ctx({ "browser:identity": "incognito:11111111-2222-3333-4444-555555555555" }))).toBeUndefined();
        expect(tab.icon!(ctx({}))).toBeUndefined();
    });

    it("marks a profile tab and an Incognito tab, and leaves Personal bare", () => {
        expect(tab.badge!(ctx({ "browser:identity": "profile:work" }))?.label).toBe("Work");
        expect(tab.badge!(ctx({ "browser:identity": "incognito:11111111-2222-3333-4444-555555555555" }))?.label).toBe("Incognito");
        expect(tab.badge!(ctx({}))).toBeUndefined();
    });

    it("shows a moon, with no mark, while the pane sleeps", () => {
        const c = ctx({ "browser:identity": "profile:work", "browser:sleep": { since: 1, reason: "manual", url: "https://example.com/" } });
        expect(tab.icon!(c)).toEqual({ kind: "fa", name: "moon" });
        expect(tab.badge!(c)).toBeUndefined();
    });
});
