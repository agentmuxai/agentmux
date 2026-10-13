// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const setMeta = vi.fn<(...args: unknown[]) => Promise<void>>(async () => {});
const store = vi.hoisted(() => ({ block: undefined as unknown }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetMetaCommand: (...a: unknown[]) => setMeta(...a) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { tab: true } }));
vi.mock("./mos", () => ({
    makeORef: (t: string, id: string) => `${t}:${id}`,
    getObjectValue: () => store.block,
}));

import { blockEnv, mergeBlockEnv, setBlockMeta } from "./block-meta";

beforeEach(() => {
    setMeta.mockClear();
    store.block = undefined;
});

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

describe("mergeBlockEnv", () => {
    // REPORT_FORK_AGENT_ISOLATION_2026_10_12.md F2: a meta write replaces
    // `cmd:env` whole, so writing only the auth dir dropped the launch's id.
    it("keeps the block's other variables and sets the given ones", async () => {
        store.block = { meta: { "cmd:env": { AGENTMUX_AGENT_ID: "loap-3", CLAUDE_CONFIG_DIR: "/old" } } };
        await mergeBlockEnv("b1", { CLAUDE_CONFIG_DIR: "/new" });
        expect(setMeta).toHaveBeenCalledWith(
            { tab: true },
            { oref: "block:b1", meta: { "cmd:env": { AGENTMUX_AGENT_ID: "loap-3", CLAUDE_CONFIG_DIR: "/new" } } },
        );
    });

    it("works on a block with no env yet", async () => {
        await mergeBlockEnv("b1", { CLAUDE_CONFIG_DIR: "/new" });
        expect(setMeta).toHaveBeenCalledWith({ tab: true }, { oref: "block:b1", meta: { "cmd:env": { CLAUDE_CONFIG_DIR: "/new" } } });
    });
});

describe("blockEnv", () => {
    it("keeps only string values", () => {
        store.block = { meta: { "cmd:env": { A: "1", B: 2, C: null } } };
        expect(blockEnv("b1")).toEqual({ A: "1" });
    });
});
