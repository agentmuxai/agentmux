// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ActiveSubagent } from "../../swarm/swarm-model";
import type { DocumentNode, ShellNode, ToolNode } from "../types";
import { hasLiveAttachedActivity } from "./attached-task";
import { TOOL_PROMOTION_MS } from "./tool-adapter";

function mkShell(overrides: Partial<ShellNode> = {}): ShellNode {
    return {
        type: "shell",
        id: "shell-1",
        cmd: "npm run dev",
        title: "dev server",
        status: "running",
        spawnedAt: 1000,
        log: { chunks: [], totalBytes: 0 } as unknown as ShellNode["log"],
        ...overrides,
    };
}

function mkBash(overrides: Partial<ToolNode> = {}): ToolNode {
    return {
        type: "tool",
        id: "tool-1",
        tool: "Bash",
        status: "running",
        params: { command: "sleep 300" },
        collapsed: false,
        summary: "",
        timestamp: 0,
        ...overrides,
    };
}

function mkSub(overrides: Partial<ActiveSubagent> & Pick<ActiveSubagent, "agent_id">): ActiveSubagent {
    return {
        slug: "some-slug",
        parent_agent: "parent",
        parent_block_id: "block-1",
        session_id: "session-1",
        status: "active",
        spawned_at: 1000,
        last_event_at: 1000,
        event_count: 0,
        model: null,
        dispatch_id: `solo:${overrides.agent_id}`,
        display_name: null,
        ...overrides,
    };
}

describe("hasLiveAttachedActivity", () => {
    it("false with no activity sources at all", () => {
        expect(hasLiveAttachedActivity([], [], "block-1", 0)).toBe(false);
    });

    it("true while a shell node is running; false once it exits", () => {
        expect(hasLiveAttachedActivity([mkShell()], [], "block-1", 0)).toBe(true);
        expect(
            hasLiveAttachedActivity([mkShell({ status: "exited-ok", exitedAt: 2000 })], [], "block-1", 0),
        ).toBe(false);
    });

    it("true for a subagent of THIS block only", () => {
        expect(hasLiveAttachedActivity([], [mkSub({ agent_id: "a1" })], "block-1", 0)).toBe(true);
        expect(hasLiveAttachedActivity([], [mkSub({ agent_id: "a1" })], "block-2", 0)).toBe(false);
    });

    it("a running Bash call counts only after crossing the promotion threshold", () => {
        const nodes: DocumentNode[] = [mkBash({ timestamp: 1000 })];
        expect(hasLiveAttachedActivity(nodes, [], "block-1", 1000 + TOOL_PROMOTION_MS - 1)).toBe(false);
        expect(hasLiveAttachedActivity(nodes, [], "block-1", 1000 + TOOL_PROMOTION_MS)).toBe(true);
    });

    it("a finished promoted Bash call (lingering in dock retention) does NOT count as live", () => {
        const nodes: DocumentNode[] = [mkBash({ status: "success", timestamp: 1000, duration: 60 })];
        expect(hasLiveAttachedActivity(nodes, [], "block-1", 1000 + 120_000)).toBe(false);
    });
});
