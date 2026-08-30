// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ActiveSubagent } from "../../swarm/swarm-model";
import type { DocumentNode, ShellNode, ToolNode } from "../types";
import { earliestLiveAttachedStartMs, hasLiveAttachedActivity } from "./attached-task";
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
        // Not a sleep — see the same note in tool-adapter.test.ts's mkBash:
        // a whole-command sleep skips the threshold entirely.
        params: { command: "cargo test -p agentmux-srv" },
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

describe("earliestLiveAttachedStartMs", () => {
    it("null when nothing is running", () => {
        expect(earliestLiveAttachedStartMs([], [], "block-1", 0)).toBeNull();
    });

    it("returns the activity's REAL start time, not the observation time", () => {
        // A promoted Bash call has already been running ≥30s when the axis
        // first observes it — `since` must be its startedAt, so the elapsed
        // counter doesn't restart at 0 (reagent P1 on PR #2489).
        const nodes: DocumentNode[] = [mkBash({ timestamp: 1000 })];
        expect(earliestLiveAttachedStartMs(nodes, [], "block-1", 1000 + TOOL_PROMOTION_MS + 5_000)).toBe(1000);
    });

    it("returns the EARLIEST start across multiple running activities", () => {
        const nodes: DocumentNode[] = [mkShell({ id: "s1", spawnedAt: 500 })];
        const subs = [mkSub({ agent_id: "a1", spawned_at: 2000 })];
        expect(earliestLiveAttachedStartMs(nodes, subs, "block-1", 10_000)).toBe(500);
    });
});

// ── Backgrounded calls reach the axis by construction (issue #2490) ──

describe("backgrounded Bash calls (issue #2490)", () => {
    it("an accepted background launch counts as live attached work from its call time", () => {
        const bg = mkBash({
            id: "toolu_bg",
            status: "success",
            params: { command: "task dev", run_in_background: true },
            timestamp: 1000,
            duration: 0.4,
            result: { stdout: "Command running in background with ID: b12345. Output is being written to: …", stderr: "", exitCode: 0 },
        });
        // Sub-second and terminal — invisible to the duration heuristic,
        // but a declared background task must still light the axis.
        expect(hasLiveAttachedActivity([bg], [], "block-1", 2000)).toBe(true);
        expect(earliestLiveAttachedStartMs([bg], [], "block-1", 2000)).toBe(1000);
    });

    it("its task-notification ends the episode", () => {
        const bg = mkBash({
            id: "toolu_bg",
            status: "success",
            params: { command: "task dev", run_in_background: true },
            timestamp: 1000,
            duration: 0.4,
            result: { stdout: "Command running in background with ID: b12345. Output is being written to: …", stderr: "", exitCode: 0 },
        });
        const notification: DocumentNode = {
            type: "user_message",
            id: "user-1",
            message: "<task-notification>\n<tool-use-id>toolu_bg</tool-use-id>\n<status>completed</status>\n</task-notification>",
            timestamp: 900_000,
        };
        expect(hasLiveAttachedActivity([bg, notification], [], "block-1", 950_000)).toBe(false);
    });
});
