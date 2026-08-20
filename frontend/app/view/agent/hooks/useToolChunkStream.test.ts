// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { parsePidChunk } from "./useToolChunkStream";

// Phase A of docs/specs/SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md:
// bashwrap publishes an `op: "pid"` tool_chunk event only for a
// declared-background invocation. These tests cover the pure parse that
// decides whether to relay it as a BackgroundTaskPidCommand.

describe("parsePidChunk", () => {
    it("parses a well-formed pid chunk", () => {
        expect(parsePidChunk({ op: "pid", tool_id: "toolu_bg", pid: 4242, timestamp: 1 })).toEqual({
            toolId: "toolu_bg",
            pid: 4242,
        });
    });

    it("returns null for a non-pid op", () => {
        expect(parsePidChunk({ op: "chunk", tool_id: "toolu_bg", pid: 4242 })).toBeNull();
        expect(parsePidChunk({ op: "terminal", tool_id: "toolu_bg" })).toBeNull();
    });

    it("returns null when tool_id is missing or empty", () => {
        expect(parsePidChunk({ op: "pid", pid: 4242 })).toBeNull();
        expect(parsePidChunk({ op: "pid", tool_id: "", pid: 4242 })).toBeNull();
    });

    it("returns null when pid is missing or not a finite number", () => {
        expect(parsePidChunk({ op: "pid", tool_id: "toolu_bg" })).toBeNull();
        expect(parsePidChunk({ op: "pid", tool_id: "toolu_bg", pid: "4242" })).toBeNull();
        expect(parsePidChunk({ op: "pid", tool_id: "toolu_bg", pid: Number.NaN })).toBeNull();
        expect(parsePidChunk({ op: "pid", tool_id: "toolu_bg", pid: Infinity })).toBeNull();
    });

    it("returns null for a non-object payload", () => {
        expect(parsePidChunk(null)).toBeNull();
        expect(parsePidChunk(undefined)).toBeNull();
        expect(parsePidChunk("not-an-object")).toBeNull();
    });
});
