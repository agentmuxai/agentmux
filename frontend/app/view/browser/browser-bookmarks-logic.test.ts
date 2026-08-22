// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { findBookmark, toggleBookmark } from "./browser-bookmarks-logic";

function bookmark(id: string, url: string, title = "T"): BrowserBookmark {
    return { id, title, url, favicon_url: "", created_at: 0 };
}

describe("findBookmark", () => {
    it("returns undefined for an empty list", () => {
        expect(findBookmark([], "https://example.com")).toBeUndefined();
    });

    it("finds an exact URL match", () => {
        const list = [bookmark("b1", "https://example.com")];
        expect(findBookmark(list, "https://example.com")?.id).toBe("b1");
    });

    it("does not match a different URL", () => {
        const list = [bookmark("b1", "https://example.com")];
        expect(findBookmark(list, "https://example.com/other")).toBeUndefined();
    });
});

describe("toggleBookmark", () => {
    it("adds a new bookmark when the URL isn't already saved", () => {
        const next = toggleBookmark([], {
            url: "https://example.com",
            title: "Example",
            faviconUrl: "https://example.com/favicon.ico",
            newId: () => "generated-id",
            now: () => 1000,
        });
        expect(next).toEqual([
            {
                id: "generated-id",
                title: "Example",
                url: "https://example.com",
                favicon_url: "https://example.com/favicon.ico",
                created_at: 1000,
            },
        ]);
    });

    it("removes the existing bookmark when the URL is already saved (toggle off)", () => {
        const existing = [bookmark("b1", "https://example.com"), bookmark("b2", "https://other.com")];
        const next = toggleBookmark(existing, {
            url: "https://example.com",
            title: "Example",
            faviconUrl: "",
            newId: () => "unused",
            now: () => 1000,
        });
        expect(next).toEqual([bookmark("b2", "https://other.com")]);
    });

    it("never produces two entries for the same URL (repeated toggling doesn't duplicate)", () => {
        let list: BrowserBookmark[] = [];
        const opts = { url: "https://example.com", title: "Example", faviconUrl: "", newId: () => "b1", now: () => 1000 };
        list = toggleBookmark(list, opts); // add
        list = toggleBookmark(list, opts); // remove
        list = toggleBookmark(list, opts); // add again
        expect(list.filter((b) => b.url === "https://example.com")).toHaveLength(1);
    });

    it("falls back to the URL as the title when no page title is available yet", () => {
        const next = toggleBookmark([], {
            url: "https://example.com",
            title: "",
            faviconUrl: "",
            newId: () => "b1",
            now: () => 1000,
        });
        expect(next[0].title).toBe("https://example.com");
    });
});
