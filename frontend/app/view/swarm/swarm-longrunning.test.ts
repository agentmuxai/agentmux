// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

// `HistoryLoaded` fires a fire-and-forget `DockNodeStatusCommand` push for
// `muxspect dock`'s snapshot cache; there is no RPC client in a unit test.
// Stubbed rather than worked around, since this file is about the selector.
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { DockNodeStatusCommand: () => Promise.resolve() },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
import { __resetAllSlots, dispatch, registerPane } from "@/app/store/agent-document-store";
import { TOOL_PROMOTION_MS } from "@/app/view/agent/activity/tool-adapter";
import { longRunningToolRows } from "./swarm-longrunning";
import type { DocumentNode } from "@/app/view/agent/types";

afterEach(__resetAllSlots);

const START = 1_000_000;

function bash(over: Partial<DocumentNode> = {}): DocumentNode {
    return {
        type: "tool",
        id: "t1",
        tool: "Bash",
        toolName: "Bash",
        status: "running",
        timestamp: START,
        params: { command: "cargo test -p agentmux-srv" },
        collapsed: false,
        summary: "",
        ...over,
    } as DocumentNode;
}

/** Register a pane and seed its document with `nodes`. */
function paneWith(blockId: string, nodes: DocumentNode[]) {
    registerPane(blockId, () => {});
    dispatch(blockId, { type: "StreamFlush", newNodes: nodes, updatedNodes: [] }, "system");
}

describe("longRunningToolRows", () => {
    it("returns nothing for an unmounted pane — zero, never a crash", () => {
        expect(longRunningToolRows("never-registered", START)).toEqual([]);
        expect(longRunningToolRows(null, START)).toEqual([]);
    });

    it("omits a Bash call that hasn't crossed the promotion threshold", () => {
        paneWith("b1", [bash()]);
        expect(longRunningToolRows("b1", START + TOOL_PROMOTION_MS - 1)).toEqual([]);
    });

    it("includes one past the threshold", () => {
        paneWith("b1", [bash()]);
        const rows = longRunningToolRows("b1", START + TOOL_PROMOTION_MS);
        expect(rows.map((r) => r.id)).toEqual(["t1"]);
        expect(rows[0].title).toBe("cargo test -p agentmux-srv");
    });

    /** The case this bucket exists for: a bare sleep is promoted immediately by
     *  `sleep-detect.ts`, so Swarm must show it at once too — not 30s later. */
    it("includes a whole-command sleep immediately, with its countdown data", () => {
        paneWith("b1", [bash({ params: { command: "sleep 300" } })]);
        const rows = longRunningToolRows("b1", START);
        expect(rows.map((r) => r.id)).toEqual(["t1"]);
        expect(rows[0].sleepMs).toBe(300_000);
    });

    it("carries no sleepMs for a call whose remaining time isn't knowable", () => {
        paneWith("b1", [bash()]);
        expect(longRunningToolRows("b1", START + TOOL_PROMOTION_MS)[0].sleepMs).toBeUndefined();
    });

    /** Swarm answers "what is this agent on RIGHT NOW". A finished call lingers
     *  in toolActivities through the dock's retention window so its dock row can
     *  resolve in place — but it is not current work. */
    it("excludes a finished call still inside the dock's retention window", () => {
        paneWith("b1", [bash({ status: "success", duration: 45 })]);
        expect(longRunningToolRows("b1", START + 46_000)).toEqual([]);
    });

    it("orders newest-first, matching the other Swarm buckets", () => {
        paneWith("b1", [
            bash({ id: "old", timestamp: START, params: { command: "sleep 300" } }),
            bash({ id: "new", timestamp: START + 5_000, params: { command: "sleep 300" } }),
        ]);
        expect(longRunningToolRows("b1", START + 6_000).map((r) => r.id)).toEqual(["new", "old"]);
    });

    it("keeps panes independent — one agent's work never shows on another's row", () => {
        paneWith("b1", [bash({ id: "a", params: { command: "sleep 300" } })]);
        paneWith("b2", [bash({ id: "b", params: { command: "sleep 300" } })]);
        expect(longRunningToolRows("b1", START).map((r) => r.id)).toEqual(["a"]);
        expect(longRunningToolRows("b2", START).map((r) => r.id)).toEqual(["b"]);
    });
});
