// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The mark a pane bound with Open here carries, so srv leaves the expected refusal out of its history. */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setMeta = vi.fn();
let storedMeta: Record<string, unknown> = {};

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetMetaCommand: (_c: unknown, data: unknown) => {
            setMeta(data);
            return Promise.resolve();
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mos", () => ({
    makeORef: (type: string, id: string) => `${type}:${id}`,
    getMuxObjectAtom: () => () => ({ meta: storedMeta }),
}));
vi.mock("@/app/util/reveal-block", () => ({ findTabHolding: vi.fn() }));
vi.mock("@/layout/lib/layoutModelHooks", () => ({ getLayoutModelForTabById: vi.fn() }));
vi.mock("../close-agent-tab", () => ({ closeAgentTab: vi.fn() }));

import { OPEN_HERE_MARK_MS } from "./open-here";
import { markOpenHerePane, unmarkOpenHerePane } from "./open-here-mark";

describe("the Open here mark on a pane", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        setMeta.mockClear();
        storedMeta = {};
    });
    afterEach(() => vi.useRealTimers());

    it("is set on the pane's block before it is bound", async () => {
        await markOpenHerePane("blk-1");
        expect(setMeta).toHaveBeenCalledWith({ oref: "block:blk-1", meta: { "agent:open_here": true } });
    });

    it("lapses with the window's own mark when the agent was free and nothing refused the pane", async () => {
        await markOpenHerePane("blk-1");
        storedMeta = { "agent:open_here": true };
        setMeta.mockClear();
        await vi.advanceTimersByTimeAsync(OPEN_HERE_MARK_MS + 1);
        expect(setMeta).toHaveBeenCalledWith({ oref: "block:blk-1", meta: { "agent:open_here": null } });
    });

    it("writes nothing to drop a mark srv has already cleared", async () => {
        await markOpenHerePane("blk-1");
        setMeta.mockClear();
        await vi.advanceTimersByTimeAsync(OPEN_HERE_MARK_MS + 1);
        expect(setMeta).not.toHaveBeenCalled();
    });

    it("is dropped when the open failed", async () => {
        storedMeta = { "agent:open_here": true };
        await unmarkOpenHerePane("blk-2");
        expect(setMeta).toHaveBeenCalledWith({ oref: "block:blk-2", meta: { "agent:open_here": null } });
    });
});
