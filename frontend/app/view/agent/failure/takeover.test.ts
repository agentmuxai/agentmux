// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

const AgentTakeoverCommand = vi.fn(async (_client: unknown, _data: unknown, _opts?: { timeout?: number }) => ({ released: true }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AgentTakeoverCommand: (...args: any[]) => (AgentTakeoverCommand as any)(...args) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { requestAgentTakeover } from "./takeover";

describe("requestAgentTakeover", () => {
    it("asks srv, over the window's socket, to take the block's agent over", async () => {
        const call = vi.fn(async () => ({ released: true, from_channel: "stable" }));
        const r = await requestAgentTakeover("block-1", { call });
        expect(r).toEqual({ released: true, fromChannel: "stable" });
        expect(call).toHaveBeenCalledWith("block-1");
    });

    it("gives srv long enough for the other instance to let go", async () => {
        await requestAgentTakeover("block-9");
        const [, data, opts] = AgentTakeoverCommand.mock.calls[0];
        expect(data).toEqual({ block_id: "block-9" });
        expect(opts?.timeout).toBeGreaterThanOrEqual(60_000);
    });

    it("reports nothing to take over as released: false", async () => {
        const r = await requestAgentTakeover("b", { call: async () => ({ released: false, from_channel: null }) });
        expect(r).toEqual({ released: false, fromChannel: undefined });
    });

    it("rejects with the srv's own reason", async () => {
        const call = async () => {
            throw new Error("The AgentMux instance running Agent3 (channel x) is too old to hand it over.");
        };
        await expect(requestAgentTakeover("b", { call })).rejects.toThrow("too old to hand it over");
    });
});
