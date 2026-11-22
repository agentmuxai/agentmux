// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ atoms: { muxWindow: () => undefined }, workspace: () => undefined }));

import type { DeepLinkOpenEvent } from "@/types/rpc/DeepLinkOpenEvent";
import { carryOut, isMine, targetOf, wireOutcome, type BridgeDeps } from "./agent-open-bridge";

const NOW = 1_000_000;

function ev(over: Partial<DeepLinkOpenEvent> = {}): DeepLinkOpenEvent {
    return {
        request_id: "r1",
        link: "agentmux://settings/recording",
        agent_id: "sorrel",
        caller_block_id: "conv",
        tab_id: "t1",
        window_id: "w1",
        target: "beside",
        expires_at_ms: BigInt(NOW + 5000),
        ...over,
    };
}

function deps(over: Partial<BridgeDeps> = {}): BridgeDeps & { answered: [string, unknown][] } {
    const answered: [string, unknown][] = [];
    return {
        answered,
        now: () => NOW,
        windowId: () => "w1",
        holdsTab: (t) => t === "t1",
        lastUsed: () => false,
        open: vi.fn(async () => ({ outcome: "opened" as const, label: "Settings › Recording", blockId: "b9" })),
        answer: async (id, out) => void answered.push([id, out]),
        ...over,
    };
}

describe("the window's side of an agent's open", () => {
    it("carries out an open for this window and answers srv with the outcome", async () => {
        const d = deps();
        await carryOut(ev(), d, new Set());
        expect(d.open).toHaveBeenCalledWith({ link: "agentmux://settings/recording", target: { kind: "beside" }, agentId: "sorrel", pane: "conv", requestId: "r1" });
        expect(d.answered).toEqual([["r1", { outcome: "opened", label: "Settings › Recording", block_id: "b9" }]]);
    });

    it("leaves an open for another window, an expired one, and one it already carried out", async () => {
        const d = deps();
        const seen = new Set<string>();
        await carryOut(ev({ window_id: "w2" }), d, seen);
        await carryOut(ev({ expires_at_ms: BigInt(NOW) }), d, seen);
        await carryOut(ev(), d, seen);
        await carryOut(ev(), d, seen);
        expect(d.open).toHaveBeenCalledTimes(1);
    });

    it("finds its window by the tab when srv named none, else by the person's last use", () => {
        expect(isMine(ev({ window_id: undefined }), deps())).toBe(true);
        expect(isMine(ev({ window_id: undefined, tab_id: "t9" }), deps())).toBe(false);
        expect(isMine(ev({ window_id: undefined, tab_id: undefined }), deps({ lastUsed: () => true }))).toBe(true);
        expect(isMine(ev({ window_id: undefined, tab_id: undefined }), deps())).toBe(false);
    });

    it("reads srv's target", () => {
        expect(targetOf({ target: "pane:b3", caller_block_id: "conv" })).toEqual({ kind: "pane", blockId: "b3" });
        expect(targetOf({ target: "new", caller_block_id: "conv" })).toEqual({ kind: "new" });
        expect(targetOf({ target: "beside", caller_block_id: "conv", tab_id: "t1" })).toEqual({ kind: "beside" });
        // A caller with no pane opens in the tab srv chose for it.
        expect(targetOf({ target: "beside", tab_id: "t1" })).toEqual({ kind: "tab", tabId: "t1" });
    });

    it("answers a refusal when the open throws, never nothing", async () => {
        const d = deps({ open: vi.fn(async () => Promise.reject(new Error("no layout"))) });
        await carryOut(ev(), d, new Set());
        expect(d.answered[0][1]).toMatchObject({ outcome: "refused", reason_code: "not_open", reason: "no layout" });
    });

    it("writes the reason and the place as srv records them", () => {
        expect(
            wireOutcome({ outcome: "opened", label: "Vault", blockId: "b1", place: "missing", reason: { code: "would_replace", text: "beside" }, remedy: { text: "turn it on" } })
        ).toEqual({ outcome: "opened", label: "Vault", block_id: "b1", place: "missing", reason_code: "would_replace", reason: "beside", remedy: "turn it on" });
    });
});
