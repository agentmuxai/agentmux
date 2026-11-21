// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a collapsed node says of itself: the App query's `memory` and `presence` queries, and the Task block when it
 * adds a note to a task instead of filing one.
 */

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { FlowNode } from "./drone-types";
import { nodeSummary } from "./drone-view";

const node = (data: Record<string, unknown>): FlowNode => ({ id: "n1", position: { x: 0, y: 0 }, data }) as FlowNode;

describe("nodeSummary", () => {
    it("names the Memory entry an App query reads, or says there is none", () => {
        expect(nodeSummary(node({ kind: "app_query", query: "memory", entry: "Release rules" }))).toBe("memory Release rules");
        expect(nodeSummary(node({ kind: "app_query", query: "memory" }))).toBe("memory <no entry>");
    });

    it("says what a presence query looks at, leaving out the filters that are empty", () => {
        expect(nodeSummary(node({ kind: "app_query", query: "presence", repo: "a/b", branch: " ", path: "src" }))).toBe("who is on a/b src");
        expect(nodeSummary(node({ kind: "app_query", query: "presence" }))).toBe("who is on <nothing>");
    });

    it("leaves the older queries as they were", () => {
        expect(nodeSummary(node({ kind: "app_query", query: "agent", agent: "clamk" }))).toBe("agent clamk");
        expect(nodeSummary(node({ kind: "app_query", query: "tasks" }))).toBe("tasks");
    });

    it("shows a Task block that adds a note as a note, and a cleared note field as a task again", () => {
        expect(nodeSummary(node({ kind: "task", title: "Nightly red", note_task: "12", note: "again" }))).toBe("note on task 12 · again");
        expect(nodeSummary(node({ kind: "task", title: "Nightly red", note_task: 12, note: "again" }))).toBe("note on task 12 · again");
        expect(nodeSummary(node({ kind: "task", title: "Nightly red", note_task: "  ", note: "again" }))).toBe("Nightly red");
        expect(nodeSummary(node({ kind: "task", title: "Nightly red" }))).toBe("Nightly red");
    });
});
