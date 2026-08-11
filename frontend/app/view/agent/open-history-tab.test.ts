// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression test for the concurrent-open race flagged independently by
 * both reagent (P2) and codex (P2) on PR #2539: two near-simultaneous
 * `openOrFocusHistoryTab` calls (a double-click, or the link row and the
 * context menu entry both firing before the first `pane.open` RPC
 * resolves) must not each push a duplicate history tab.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
// vi.mock calls below are hoisted by vitest above this import, so the
// module under test picks up the mocked dependencies despite the normal
// (not dynamic) import order here — avoids a top-level `await import()`,
// which `tsc --noEmit` rejects under this repo's module/target config even
// though vitest's own esbuild transform handles it fine at test time.
import { openOrFocusHistoryTab } from "./open-history-tab";

const getNodeByBlockId = vi.fn();
const pushBlockOntoStack = vi.fn();
const setActiveBlockInStack = vi.fn();
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({ getNodeByBlockId }),
    pushBlockOntoStack: (...args: unknown[]) => pushBlockOntoStack(...args),
    setActiveBlockInStack: (...args: unknown[]) => setActiveBlockInStack(...args),
}));

const rpcCall = vi.fn();
vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: { rpcCall: (...args: unknown[]) => rpcCall(...args) },
}));

vi.mock("@/app/store/services", () => ({
    ObjectService: { DeleteBlock: vi.fn().mockResolvedValue(undefined) },
}));

const getObjectValue = vi.fn();
vi.mock("@/app/store/global", () => ({
    pushNotification: vi.fn(),
    WOS: {
        getObjectValue: (...args: unknown[]) => getObjectValue(...args),
        makeORef: (kind: string, id: string) => `${kind}:${id}`,
    },
}));

describe("openOrFocusHistoryTab concurrency (reagent P2 / codex P2 on PR #2539)", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        getNodeByBlockId.mockReturnValue({ id: "node-1", data: { blockStack: ["live-block"] } });
        getObjectValue.mockReturnValue(undefined); // no existing history tab in the stack
    });

    afterEach(() => {
        vi.restoreAllMocks();
    });

    it("two concurrent calls for the same pane/agent issue only ONE pane.open RPC and push only ONE block", async () => {
        let resolveRpc: (v: unknown) => void;
        rpcCall.mockReturnValue(new Promise((resolve) => { resolveRpc = resolve; }));

        const first = openOrFocusHistoryTab({ currentBlockId: "live-block", agentId: "agent-1" });
        const second = openOrFocusHistoryTab({ currentBlockId: "live-block", agentId: "agent-1" });

        // Both calls are in flight, neither has resolved yet — the RPC must
        // have been issued exactly once (the actual bug: it used to fire
        // twice here).
        expect(rpcCall).toHaveBeenCalledTimes(1);

        resolveRpc!({ block_id: "history-block-1" });
        await Promise.all([first, second]);

        expect(rpcCall).toHaveBeenCalledTimes(1);
        expect(pushBlockOntoStack).toHaveBeenCalledTimes(1);
        expect(pushBlockOntoStack).toHaveBeenCalledWith(expect.anything(), "node-1", "history-block-1");
    });

    it("a call AFTER the first fully resolves is independent — re-opening (already-open case) still works", async () => {
        rpcCall.mockResolvedValue({ block_id: "history-block-1" });
        await openOrFocusHistoryTab({ currentBlockId: "live-block", agentId: "agent-1" });
        expect(rpcCall).toHaveBeenCalledTimes(1);

        // Now the tab exists — a later, non-overlapping call must focus it,
        // not open a second one.
        getObjectValue.mockImplementation((oref: string) =>
            oref === "block:history-block-1" ? { meta: { "agent:historyTabFor": "agent-1" } } : undefined,
        );
        getNodeByBlockId.mockReturnValue({ id: "node-1", data: { blockStack: ["live-block", "history-block-1"] } });

        await openOrFocusHistoryTab({ currentBlockId: "live-block", agentId: "agent-1" });
        expect(rpcCall).toHaveBeenCalledTimes(1); // still just the one from before
        expect(setActiveBlockInStack).toHaveBeenCalledWith(expect.anything(), "node-1", "history-block-1");
    });

    it("concurrent calls for DIFFERENT agents/panes are independent — each gets its own RPC", async () => {
        rpcCall.mockResolvedValue({ block_id: "history-block-x" });

        await Promise.all([
            openOrFocusHistoryTab({ currentBlockId: "live-block", agentId: "agent-1" }),
            openOrFocusHistoryTab({ currentBlockId: "live-block", agentId: "agent-2" }),
        ]);

        expect(rpcCall).toHaveBeenCalledTimes(2);
    });
});
