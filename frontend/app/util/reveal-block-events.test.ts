// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `block:reveal` receiving side — SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md §4.3.
 * Only the window srv named acts, and it acts through revealBlock (this window only) with
 * srv's tab as the hint.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const reveal = vi.fn(async (_blockId: string, _opts?: { tabId?: string }) => (order.push("reveal"), true));
let subscribed: { eventType: string; handler: (e: { data: unknown }) => void }[] = [];

const order: string[] = [];
vi.mock("@/app/store/global", () => ({
    getApi: () => ({ getWindowLabel: async () => "main", focusWindow: async (l: string) => void order.push(`raise:${l}`) }),
}));
const revealed = vi.fn(async (_d: unknown) => ({ delivered: true }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { BlockRevealedCommand: (_c: unknown, d: unknown) => revealed(d) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({
    revealBlock: async (b: string, o?: { tabId?: string }) => ((await reveal(b, o)) ? { ok: true } : { ok: false, reason: "layout-timeout" }),
}));
vi.mock("@/app/store/window-identity", () => ({ windowId: () => "w1" }));
vi.mock("@/app/store/mps", async () => {
    const { isNoPayloadMuxEvent } = await vi.importActual<typeof import("@/app/store/mps-events.generated")>("@/app/store/mps-events.generated");
    const mock = {
        muxEventSubscribe: (s: { eventType: string; handler: (e: { data: unknown }) => void }) => {
            subscribed.push(s);
            return () => {
                subscribed = subscribed.filter((x) => x !== s);
            };
        },
    };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); else if (isNoPayloadMuxEvent(eventType)) handler(null, e); } } as any);
    return { ...mock, onMuxEvent };
});

import { EVENT_BLOCK_REVEAL, handleBlockReveal, installBlockRevealEvents, shouldRevealHere } from "./reveal-block-events";

beforeEach(() => {
    reveal.mockClear();
    revealed.mockClear();
    order.length = 0;
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
        expect(reveal).toHaveBeenCalledWith("b", { tabId: "t", source: "remote-window", localOnly: true, quiet: false });
    });

    it("answers a caller that waits (its request_id), quiet itself, with how it went", async () => {
        const d = { block_id: "b", tab_id: "t", window_id: "w1", request_id: "r1" };
        expect(await handleBlockReveal(d, "w1")).toBe(true);
        expect(reveal).toHaveBeenCalledWith("b", { tabId: "t", source: "remote-window", localOnly: true, quiet: true });
        expect(revealed).toHaveBeenLastCalledWith({ request_id: "r1", ok: true, reason: undefined });
        // Raised before the pane is shown: a hidden window draws no frames for the read-back.
        expect(order.slice(0, 2)).toEqual(["raise:main", "reveal"]);
        reveal.mockResolvedValueOnce(false);
        expect(await handleBlockReveal(d, "w1")).toBe(false);
        expect(revealed).toHaveBeenLastCalledWith({ request_id: "r1", ok: false, reason: "layout-timeout" });
    });

    it("answers no one when nobody waits", async () => {
        await handleBlockReveal({ block_id: "b", tab_id: "t", window_id: "w1" }, "w1");
        expect(revealed).not.toHaveBeenCalled();
        expect(order).toEqual(["reveal"]); // and an agent's pane open does not take the window to the front
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
        expect(reveal).toHaveBeenCalledWith("b", { tabId: "t", source: "remote-window", localOnly: true, quiet: false });

        off();
        expect(subscribed).toEqual([]);
    });
});
