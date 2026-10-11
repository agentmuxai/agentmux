// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A pre-warmed window says so while it waits to be claimed, and stops saying
// so once promoted (docs/specs/PLAN_SHORTCUT_KINKS_2026_10_10.md, A5).

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ handlers: new Map<string, (payload: unknown) => void>() }));
vi.mock("@/store/global", () => ({
    getApi: () => ({
        listen: async (event: string, cb: (payload: unknown) => void) => {
            h.handlers.set(event, cb);
            return () => {};
        },
        getWindowLabel: async () => "window-pool-1",
        windows: { poolWindowReady: async () => {} },
        setWindowInitStatus: () => {},
    }),
}));
vi.mock("./startup-splash", () => ({ markPoolPromoted: vi.fn(), showTearOffSnapshot: vi.fn() }));

import { awaitPoolPromote, markPoolWaiting } from "./pool";

beforeEach(() => {
    h.handlers.clear();
    document.title = "";
    delete document.documentElement.dataset.poolWindow;
});

describe("pool window marker", () => {
    it("names a waiting pool window in its title and a data attribute", () => {
        markPoolWaiting("window");
        expect(document.title).toBe("AgentMux (pre-warmed window)");
        expect(document.documentElement.dataset.poolWindow).toBe("window");
        markPoolWaiting("pane");
        expect(document.title).toContain("pane window");
    });

    it.each(["pool:promote", "pool:new-window"])("clears the marker on %s", async (event) => {
        markPoolWaiting("window");
        const done = awaitPoolPromote();
        await vi.waitFor(() => expect(h.handlers.has(event)).toBe(true));
        h.handlers.get(event)!({ workspaceId: "ws1" });
        await done;
        expect(document.documentElement.dataset.poolWindow).toBeUndefined();
        expect(document.title).toBe("AgentMux");
    });
});
