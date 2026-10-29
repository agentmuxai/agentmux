// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { PREPARING_MAX_AGE_MS, preparingNow } from "./preparing";
import type { CliNoticeNode, DocumentNode } from "./types";

const NOW = 1_000_000_000;

const start = (state: CliNoticeNode["startState"], extra: Partial<CliNoticeNode> = {}): DocumentNode =>
    ({ type: "cli_notice", id: "start-1", kind: "container_start", provider: "claude", startState: state, timestamp: NOW - 1000, ...extra }) as DocumentNode;
const install = (state: CliNoticeNode["state"], extra: Partial<CliNoticeNode> = {}): DocumentNode =>
    ({ type: "cli_notice", id: "install-1", kind: "install", provider: "claude", version: "2.1.285", state, timestamp: NOW - 500, ...extra }) as DocumentNode;
const user = { type: "user_message", id: "u1", content: "hi", timestamp: NOW - 100 } as unknown as DocumentNode;

describe("preparingNow: a container agent's start-up in the background", () => {
    it("says where the start is while the image downloads or the container starts", () => {
        expect(preparingNow([start("pulling", { percent: 42 })], NOW)?.phase).toBe("Downloading the agent image… 42%");
        expect(preparingNow([start("starting")], NOW)?.phase).toBe("Starting the container…");
    });

    it("follows the CLI install that comes after the container is ready", () => {
        expect(preparingNow([start("ready", { seconds: 12 }), install("installing")], NOW)?.phase).toBe("Installing Claude Code 2.1.285…");
    });

    it("is gone when the start ends, ready or failed, and when the install ends", () => {
        expect(preparingNow([start("ready", { seconds: 12 })], NOW)).toBeNull();
        expect(preparingNow([start("failed", { error: "no docker" })], NOW)).toBeNull();
        expect(preparingNow([start("ready"), install("installed", { seconds: 14 })], NOW)).toBeNull();
        expect(preparingNow([start("ready"), install("failed")], NOW)).toBeNull();
    });

    it("is not shown once the agent has answered: it is already up", () => {
        const reply = { type: "agent_message", id: "a1", content: "hello", timestamp: NOW } as unknown as DocumentNode;
        expect(preparingNow([start("starting"), user, reply], NOW)).toBeNull();
    });

    it("stays while the user's first message waits for the container: sending is not being ready", () => {
        expect(preparingNow([start("pulling", { percent: 10 }), user], NOW)?.phase).toBe("Downloading the agent image… 10%");
        expect(preparingNow([start("ready"), install("installing"), user], NOW)?.phase).toBe("Installing Claude Code 2.1.285…");
        // once the start ends, a message after it changes nothing
        expect(preparingNow([start("ready"), user], NOW)).toBeNull();
    });

    it("ignores other notices after the start-up row", () => {
        const other = { type: "cli_notice", id: "mcp", kind: "mcp_health", provider: "claude", mcpState: "ok", timestamp: NOW } as DocumentNode;
        expect(preparingNow([start("starting"), other], NOW)?.phase).toBe("Starting the container…");
    });

    it("gives up on a start-up row that has been in progress for longer than a start can take", () => {
        expect(preparingNow([start("pulling", { timestamp: NOW - PREPARING_MAX_AGE_MS - 1 })], NOW)).toBeNull();
        expect(preparingNow([start("pulling", { timestamp: NOW - PREPARING_MAX_AGE_MS + 1000 })], NOW)).not.toBeNull();
    });

    it("is null for an empty conversation", () => {
        expect(preparingNow([], NOW)).toBeNull();
    });
});
