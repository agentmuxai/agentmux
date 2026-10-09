// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { createTestProgressTracker } from "./test-progress";
import { createThinkingHeadlineTracker, headlineOf } from "./thinking-headline";

describe("test progress from live output", () => {
    it("cargo test: counts tests against the total, then the result", () => {
        const t = createTestProgressTracker();
        expect(t("   Compiling agentmux-srv v0.59\n")).toBeNull();
        expect(t("running 3 tests\n")).toBeNull();
        expect(t("test a::one ... ok\ntest a::two ... FAILED\n")).toBe("2/3, 1 failed");
        expect(t("test a::three ... ok\n")).toBe("3/3, 1 failed");
        expect(t("test result: FAILED. 2 passed; 1 failed; 0 ignored\n")).toBe("2 passed, 1 failed");
    });

    it("vitest, jest and pytest summaries; pytest's percentage while it runs", () => {
        expect(createTestProgressTracker()("\x1b[2m Tests \x1b[22m 5616 passed (5616)\n")).toBe("5616 passed");
        expect(createTestProgressTracker()(" Tests  2 failed | 40 passed (42)\n")).toBe("40 passed, 2 failed");
        expect(createTestProgressTracker()("Tests:       1 failed, 41 passed, 42 total\n")).toBe("1 failed, 41 passed");
        const py = createTestProgressTracker();
        expect(py("tests/test_x.py ....F..                         [ 45%]\n")).toBe("45%");
        expect(py("======== 41 passed, 2 failed in 3.21s ========\n")).toBe("41 passed, 2 failed");
    });

    it("says nothing for output it doesn't know, and nothing twice", () => {
        const t = createTestProgressTracker();
        expect(t("building…\n100% done\n")).toBeNull();
        expect(t("tests/a.py ..   [ 50%]\n")).toBe("50%");
        expect(t("tests/a.py ..   [ 50%]\n")).toBeNull();
    });
});

describe("the thinking headline", () => {
    it("a bold heading wins; else the first sentence; else the start of a long run", () => {
        expect(headlineOf("**Weighing the join rule**\n\nThe ledger…")).toBe("Weighing the join rule");
        expect(headlineOf("**Weighing the jo")).toBeNull();
        expect(headlineOf("The user wants the timer per turn. Let me look.")).toBe("The user wants the timer per turn.");
        expect(headlineOf("Short.")).toBeNull();
        expect(headlineOf("x".repeat(200))?.length).toBe(80);
    });

    it("reads the main agent's thinking deltas once per thinking block", () => {
        const delta = (thinking: string, parent?: string) => ({
            type: "stream_event",
            parent_tool_use_id: parent,
            event: { type: "content_block_delta", delta: { type: "thinking_delta", thinking } },
        });
        const start = (type: string) => ({ type: "stream_event", event: { type: "content_block_start", content_block: { type } } });
        const t = createThinkingHeadlineTracker();
        expect(t(start("thinking"))).toBeNull();
        expect(t(delta("I should check whether the "))).toBeNull();
        expect(t(delta("ledger joins a queued flush. Then"))).toBe("I should check whether the ledger joins a queued flush.");
        expect(t(delta(" more text."))).toBeNull(); // once per block
        expect(t(start("thinking"))).toBeNull();
        expect(t(delta("**Next idea**", "toolu_sub"))).toBeNull(); // a subagent's
        expect(t(delta("**Next idea**"))).toBe("Next idea");
    });
});
