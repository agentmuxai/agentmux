// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { ToolCorrelator, wrapOutput } from "./tool-correlation";

describe("ToolCorrelator", () => {
    it("call() registers id → name and emits the agnostic tool_call", () => {
        const t = new ToolCorrelator();
        expect(t.call("Shell", "tc_1", { command: "ls" })).toEqual({
            type: "tool_call",
            tool: "Shell",
            id: "tc_1",
            params: { command: "ls" },
        });
    });

    it("result() resolves the tool name from the prior call", () => {
        const t = new ToolCorrelator();
        t.call("Shell", "tc_1", {});
        expect(t.result("tc_1", "success", { output: "ok" })).toEqual({
            type: "tool_result",
            tool: "Shell",
            id: "tc_1",
            status: "success",
            result: { output: "ok" },
        });
    });

    it("result() falls back to 'unknown' when the call was never seen", () => {
        const t = new ToolCorrelator();
        expect(t.result("missing", "failed", {}).tool).toBe("unknown");
    });

    it("result() honours a custom fallback name (the acp `params.toolName` case)", () => {
        const t = new ToolCorrelator();
        expect(t.result("missing", "success", {}, "Edit").tool).toBe("Edit");
        // A registered name still wins over the fallback.
        t.call("Shell", "tc_1", {});
        expect(t.result("tc_1", "success", {}, "Edit").tool).toBe("Shell");
    });

    it("reset() forgets all correlations", () => {
        const t = new ToolCorrelator();
        t.call("Shell", "tc_1", {});
        t.reset();
        expect(t.result("tc_1", "success", {}).tool).toBe("unknown");
    });
});

describe("wrapOutput", () => {
    it("wraps a string as { output }", () => {
        expect(wrapOutput("hello")).toEqual({ output: "hello" });
        expect(wrapOutput("")).toEqual({ output: "" });
    });

    it("passes a structured object through unchanged", () => {
        const obj = { stdout: "a", stderr: "b", exit: 0 };
        expect(wrapOutput(obj)).toBe(obj);
    });
});
