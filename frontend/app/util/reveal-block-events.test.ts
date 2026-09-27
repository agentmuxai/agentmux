// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `block:reveal` receiving side — SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md §4.3.
 * Only the window srv named acts, and it acts through revealBlockLocally with
 * srv's tab as the hint.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const reveal = vi.fn(async (_blockId: string, _opts?: { tabId?: string }) => true);
let subscribed: { eventType: string; handler: (e: { data: unknown }) => void }[] = [];

vi.mock("@/app/util/reveal-block", () => ({ revealBlockLocally: (b: string, o?: { tabId?: string }) => reveal(b, o) }));
vi.mock("@/app/store/window-identity", () => ({ windowId: () => "w1" }));
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (s: { eventType: string; handler: (e: { data: unknown }) => void }) => {
        subscribed.push(s);
        return () => {
            subscribed = subscribed.filter((x) => x !== s);
        };
    },
}));

import { EVENT_BLOCK_REVEAL, handleBlockReveal, installBlockRevealEvents, shouldRevealHere } from "./reveal-block-events";

beforeEach(() => {
    reveal.mockClear();
    subscribed = [];
});

describe("block:reveal handler", () => {
    it("acts only when window_id names this window", () => {
        const d = { block_id: "b", tab_id: "t", window_id: "w1" };
        expect(shouldRevealHere(d, "w1")).toBe(true);
        expect(shouldRevealHere({ ...d, window_id: "w2" }, "w1")).toBe(false);
        expect(shouldRevealHere({ ...d, window_id: undefined }, "w1")).toBe(false);
        expect(shouldRevealHere({ ...d, block_id: "" }, "w1")).toBe(false);
        expect(shouldRevealHere(d, "")).toBe(false);
        expect(shouldRevealHere(undefined, "w1")).toBe(false);
    });

    it("reveals the block locally with srv's tab as the hint", async () => {
        expect(await handleBlockReveal({ block_id: "b", tab_id: "t", window_id: "w1" }, "w1")).toBe(true);
        expect(reveal).toHaveBeenCalledWith("b", { tabId: "t" });
    });

    it("another window's event does nothing here", async () => {
        expect(await handleBlockReveal({ block_id: "b", tab_id: "t", window_id: "w2" }, "w1")).toBe(false);
        expect(reveal).not.toHaveBeenCalled();
    });

    it("installs one subscription per window, routed to this window's id", async () => {
        const off = installBlockRevealEvents();
        installBlockRevealEvents(); // second call is a no-op
        expect(subscribed.map((s) => s.eventType)).toEqual([EVENT_BLOCK_REVEAL]);

        subscribed[0].handler({ data: { block_id: "x", tab_id: "t", window_id: "w2" } });
        subscribed[0].handler({ data: { block_id: "b", tab_id: "t", window_id: "w1" } });
        await vi.waitFor(() => expect(reveal).toHaveBeenCalledTimes(1));
        expect(reveal).toHaveBeenCalledWith("b", { tabId: "t" });

        off();
        expect(subscribed).toEqual([]);
    });
});
