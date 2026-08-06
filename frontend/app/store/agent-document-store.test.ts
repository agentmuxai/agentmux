// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Regression test for reagentx P1 on PR #2432: the orphan-scrub paths
// (SessionEnd/HistoryLoaded/HistoryRestored/ScrubOrphanedInProgress)
// resolve stuck ToolNodes purely client-side — `dispatch()` is the one
// choke point every one of them funnels through, so this pins that a
// `docknodestatus` delta actually gets pushed for each resolved node,
// not just that the reducer's local state/events look right (already
// covered in reducer.test.ts).

import { describe, test, expect, beforeEach, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        DockNodeStatusCommand: vi.fn().mockResolvedValue(undefined),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import { dispatch, registerPane, __resetAllSlots } from "./agent-document-store";
import type { DocumentNode, ToolNode } from "../view/agent/types";

const blockId = "block-1";

const runningTool = (id: string): ToolNode => ({
    type: "tool",
    id,
    tool: "Bash",
    params: { command: "sleep 90" },
    status: "running",
    collapsed: false,
    summary: "🔧 Bash sleep 90",
});

describe("agent-document-store dispatch — dock node status push", () => {
    let setterCalls: DocumentNode[][] = [];

    beforeEach(() => {
        vi.clearAllMocks();
        __resetAllSlots();
        setterCalls = [];
        registerPane(blockId, (nodes) => setterCalls.push(nodes));
    });

    test("ScrubOrphanedInProgress pushes a docknodestatus delta for each resolved tool node", () => {
        dispatch(blockId, { type: "StreamFlush", newNodes: [runningTool("t1")], updatedNodes: [] });
        vi.clearAllMocks(); // only care about the scrub's own push, not StreamFlush's (unrelated path)

        dispatch(blockId, { type: "ScrubOrphanedInProgress", at: 9999 });

        expect(RpcApi.DockNodeStatusCommand).toHaveBeenCalledTimes(1);
        expect(RpcApi.DockNodeStatusCommand).toHaveBeenCalledWith(
            {},
            { blockid: blockId, node_id: "t1", tool_name: "Bash", status: "canceled" },
        );
    });

    test("SessionEnd pushes a delta for an orphaned running tool too", () => {
        dispatch(blockId, { type: "StreamFlush", newNodes: [runningTool("k1")], updatedNodes: [] });
        vi.clearAllMocks();

        dispatch(blockId, { type: "SessionEnd", at: 5000 });

        expect(RpcApi.DockNodeStatusCommand).toHaveBeenCalledWith(
            {},
            { blockid: blockId, node_id: "k1", tool_name: "Bash", status: "canceled" },
        );
    });

    test("no push when the scrub finds nothing orphaned (idempotent)", () => {
        dispatch(blockId, { type: "ScrubOrphanedInProgress", at: 1000 });
        expect(RpcApi.DockNodeStatusCommand).not.toHaveBeenCalled();
    });

    test("an ordinary StreamFlush (no scrub) does not trigger the scrub push path", () => {
        dispatch(blockId, { type: "StreamFlush", newNodes: [runningTool("t1")], updatedNodes: [] });
        // StreamFlush itself never emits "orphans-scrubbed" — this is purely
        // exercising that pushResolvedDockNodes only reacts to that event
        // type, not any dispatch that happens to touch a tool node. The
        // separate streaming-path push (stream-flush-queue.ts) is a
        // different call site, not under test here.
        expect(RpcApi.DockNodeStatusCommand).not.toHaveBeenCalled();
    });
});
