// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    closed: [] as string[],
    answer: true as boolean | Error,
    notes: [] as Array<{ title: string; message: string; type: string }>,
}));

vi.mock("@/app/util/close-own-tab", () => ({
    closeOwnTab: async (blockId: string) => {
        h.closed.push(blockId);
        if (h.answer instanceof Error) throw h.answer;
        return h.answer;
    },
}));
vi.mock("@/app/store/global", () => ({
    pushNotification: (n: { title: string; message: string; type: string }) => h.notes.push(n),
}));

import { closeBrowserTab } from "./browser-close-tab";

const model = { blockId: "block-7" } as Parameters<typeof closeBrowserTab>[0];

beforeEach(() => {
    h.closed = [];
    h.answer = true;
    h.notes = [];
});

describe("closing a browser pane's tab", () => {
    it("closes this tab and says nothing", async () => {
        await closeBrowserTab(model);
        expect(h.closed).toEqual(["block-7"]);
        expect(h.notes).toEqual([]);
    });

    it("keeps the pane's only tab and says how to close the pane", async () => {
        h.answer = false;
        await closeBrowserTab(model);
        expect(h.notes.map((n) => [n.title, n.message, n.type])).toEqual([
            ["This is the pane's only tab", "Use the pane's × to close it.", "info"],
        ]);
    });

    it("says why when the close fails", async () => {
        h.answer = new Error("the pane is gone");
        await closeBrowserTab(model);
        expect(h.notes.map((n) => [n.title, n.message, n.type])).toEqual([["Couldn't close the tab", "the pane is gone", "error"]]);
    });
});
