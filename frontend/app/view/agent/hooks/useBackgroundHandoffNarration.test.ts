// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { DocumentNode, ToolNode } from "../types";
import { handoffNarrations, newHandoffState, RECENT_MS } from "./useBackgroundHandoffNarration";

const NOW = 1_000_000;

const launch = (over: Partial<ToolNode> = {}, description = "Wait for v0.60.10 to publish"): ToolNode => ({
    type: "tool",
    id: "toolu_bg1",
    tool: "Bash",
    status: "success",
    params: { command: "for i in $(seq 1 40); do sleep 5; done", run_in_background: true, ...(description ? { description } : {}) },
    collapsed: false,
    summary: "",
    timestamp: NOW - 2000,
    duration: 0.4,
    result: { stdout: "Command running in background with ID: b1. Output is being written to: x", stderr: "", exitCode: 0 },
    ...over,
});

const finished = (id: string, status: string): DocumentNode =>
    ({
        type: "user_message",
        id: `n-${id}`,
        message: `<task-notification>\n<task-id>b1</task-id>\n<tool-use-id>${id}</tool-use-id>\n<status>${status}</status>\n</task-notification>`,
        timestamp: NOW + 5000,
    }) as unknown as DocumentNode;

describe("handoffNarrations", () => {
    it("says what moved to the background, in the model's words, and that the agent is free", () => {
        const out = handoffNarrations([launch()], newHandoffState(), NOW);
        expect(out).toHaveLength(1);
        expect(out[0].text).toBe("Moved to the background: Wait for v0.60.10 to publish. The agent is free; it picks this up when it finishes.");
        expect(out[0].type).toBe("ambient_narration");
    });

    it("says it once", () => {
        const state = newHandoffState();
        expect(handoffNarrations([launch()], state, NOW)).toHaveLength(1);
        expect(handoffNarrations([launch()], state, NOW + 1000)).toHaveLength(0);
    });

    it("says nothing for a call with no description (the model-written line covers it), a replayed launch, or an ordinary call", () => {
        expect(handoffNarrations([launch({}, "")], newHandoffState(), NOW)).toHaveLength(0);
        expect(handoffNarrations([launch({ timestamp: NOW - RECENT_MS - 1 })], newHandoffState(), NOW)).toHaveLength(0);
        expect(handoffNarrations([launch({ params: { command: "ls", description: "List" } })], newHandoffState(), NOW)).toHaveLength(0);
    });

    it("says when it finished, failed or stopped, only for a launch it narrated", () => {
        const state = newHandoffState();
        handoffNarrations([launch()], state, NOW);
        const done = handoffNarrations([launch(), finished("toolu_bg1", "completed")], state, NOW + 6000);
        expect(done.map((n) => n.text)).toEqual(["Finished in the background: Wait for v0.60.10 to publish."]);
        expect(handoffNarrations([launch(), finished("toolu_bg1", "completed")], state, NOW + 7000)).toHaveLength(0);

        const failed = newHandoffState();
        handoffNarrations([launch()], failed, NOW);
        expect(handoffNarrations([launch(), finished("toolu_bg1", "failed")], failed, NOW + 6000)[0].text).toBe(
            "Failed in the background: Wait for v0.60.10 to publish."
        );

        const stranger = newHandoffState();
        expect(handoffNarrations([finished("toolu_other", "completed")], stranger, NOW)).toHaveLength(0);
    });
});

describe("handoffNarrations — cost", () => {
    it("does not look past the recent tail of a long document", () => {
        const old = Array.from({ length: 5000 }, (_, i) => ({ type: "text", id: `old-${i}`, timestamp: NOW - 3_600_000 + i }) as unknown as DocumentNode);
        let reads = 0;
        const counted = old.map(
            (n) =>
                new Proxy(n, {
                    get(target, key, receiver) {
                        if (key === "type") reads++;
                        return Reflect.get(target, key, receiver);
                    },
                })
        );
        const out = handoffNarrations([...counted, launch()], newHandoffState(), NOW);
        expect(out).toHaveLength(1);
        expect(reads).toBe(0);
    });
});
