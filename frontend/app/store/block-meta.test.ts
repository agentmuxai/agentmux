// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

const setMeta = vi.fn<(...args: unknown[]) => Promise<void>>(async () => {});
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetMetaCommand: (...a: unknown[]) => setMeta(...a) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { tab: true } }));
vi.mock("./mos", () => ({ makeORef: (t: string, id: string) => `${t}:${id}` }));

import { setBlockMeta } from "./block-meta";

describe("setBlockMeta", () => {
    it("sends one SetMetaCommand for the block's oref, through the tab client", async () => {
        await setBlockMeta("b1", { "term:zoom": null });
        expect(setMeta).toHaveBeenCalledWith({ tab: true }, { oref: "block:b1", meta: { "term:zoom": null } });
    });

    it("returns the command's promise, so callers can await or catch it", async () => {
        setMeta.mockRejectedValueOnce(new Error("down"));
        await expect(setBlockMeta("b1", {})).rejects.toThrow("down");
    });
});
