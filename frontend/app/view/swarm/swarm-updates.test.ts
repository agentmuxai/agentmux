// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { AgentProgress } from "./swarm-model";
import { sameProgress, withAgentProgress, withAgentStatus } from "./swarm-updates";

const todo = (text: string, status = "pending") => ({ text, status }) as unknown as AgentProgress["todos"][number];
const progress = (over: Partial<AgentProgress> = {}): AgentProgress => ({
    todos: [todo("a"), todo("b", "in_progress")],
    todosTruncated: 0,
    todosPartial: false,
    currentTool: "Bash",
    ...over,
});

describe("withAgentStatus", () => {
    it("returns the same Map when the block already has that status", () => {
        const prev = new Map<string, "running" | "idle">([["b1", "running"]]);
        expect(withAgentStatus(prev, "b1", "running")).toBe(prev);
    });

    it("returns a new Map, leaving the old one alone, when it changes or the block is new", () => {
        const prev = new Map<string, "running" | "idle">([["b1", "running"]]);
        const changed = withAgentStatus(prev, "b1", "idle");
        expect(changed).not.toBe(prev);
        expect(changed.get("b1")).toBe("idle");
        expect(prev.get("b1")).toBe("running");
        expect(withAgentStatus(prev, "b2", "idle").get("b2")).toBe("idle");
    });
});

describe("withAgentProgress", () => {
    it("returns the same Map for the same progress again, even as a new object", () => {
        const prev = new Map([["b1", progress()]]);
        expect(withAgentProgress(prev, "b1", progress())).toBe(prev);
    });

    it("returns a new Map when anything the Swarm shows changed", () => {
        const prev = new Map([["b1", progress()]]);
        for (const next of [
            progress({ currentTool: "Read" }),
            progress({ currentTool: null }),
            progress({ todosTruncated: 2 }),
            progress({ todosPartial: true }),
            progress({ todos: [todo("a"), todo("b", "completed")] }),
            progress({ todos: [todo("a")] }),
        ]) {
            const out = withAgentProgress(prev, "b1", next);
            expect(out).not.toBe(prev);
            expect(out.get("b1")).toBe(next);
        }
        expect(prev.get("b1")).toEqual(progress());
    });

    it("a block with no progress yet gets its first", () => {
        const prev = new Map<string, AgentProgress>();
        const out = withAgentProgress(prev, "b1", progress());
        expect(out).not.toBe(prev);
        expect(out.get("b1")).toEqual(progress());
        expect(sameProgress(undefined, progress())).toBe(false);
    });
});
