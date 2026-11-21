// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";

async function load() {
    vi.resetModules();
    return import("./palette-recent");
}

beforeEach(() => {
    localStorage.clear();
});

describe("the recently used commands", () => {
    it("remembers the newest first, a repeat moving to the front once", async () => {
        const r = await load();
        r.rememberCommand("a");
        r.rememberCommand("b");
        r.rememberCommand("a");
        expect(r.recentCommandIds()).toEqual(["a", "b"]);
    });

    it("keeps the last 20", async () => {
        const r = await load();
        for (let i = 0; i < 25; i++) r.rememberCommand(`c${i}`);
        expect(r.recentCommandIds()).toHaveLength(20);
        expect(r.recentCommandIds()[0]).toBe("c24");
        expect(r.recentCommandIds()).not.toContain("c4");
    });

    it("survives a restart: it is read from local storage", async () => {
        const first = await load();
        first.rememberCommand("x");
        const second = await load();
        expect(second.recentCommandIds()).toEqual(["x"]);
    });

    it("ignores storage that is not a list of ids", async () => {
        localStorage.setItem("palette:recent", '{"not":"a list"}');
        expect((await load()).recentCommandIds()).toEqual([]);
        localStorage.setItem("palette:recent", "not json");
        expect((await load()).recentCommandIds()).toEqual([]);
        localStorage.setItem("palette:recent", '["ok", 3, null, "fine"]');
        expect((await load()).recentCommandIds()).toEqual(["ok", "fine"]);
    });

    it("is emptied by Clear Recently Used Commands, which shows only while there is something to clear", async () => {
        const r = await load();
        expect(r.clearRecentEntry.when?.()).toBe(false);
        r.rememberCommand("a");
        expect(r.clearRecentEntry.when?.()).toBe(true);
        await r.clearRecentEntry.execute();
        expect(r.recentCommandIds()).toEqual([]);
        expect(localStorage.getItem("palette:recent")).toBe("[]");
    });

    it("follows another window: a change to the stored list is read", async () => {
        const r = await load();
        localStorage.setItem("palette:recent", '["from-elsewhere"]');
        window.dispatchEvent(new StorageEvent("storage", { key: "palette:recent" }));
        expect(r.recentCommandIds()).toEqual(["from-elsewhere"]);
    });
});
