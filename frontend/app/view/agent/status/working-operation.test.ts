// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the Working row says the turn is waiting on, beside its slash. */

import { describe, expect, it } from "vitest";

import { IDLE_ACTIVITY, type ActivityState } from "@/app/store/agent-pane-state/types";
import { workingOperation, workingOperationText } from "./working-operation";

const NOW = 1_000_000;

const tool = (over: Partial<ActivityState["tools"][number]> = {}): ActivityState["tools"][number] => ({
    id: "t1",
    activity: { family: "bash", label: "Running cargo" },
    name: "Bash",
    command: "cargo build --release",
    startedAt: NOW - 252_000,
    ...over,
});
const withTools = (...tools: ActivityState["tools"]): ActivityState => ({ ...IDLE_ACTIVITY, tools });

describe("workingOperation", () => {
    it("is nothing when nothing long is running", () => {
        expect(workingOperation({ activity: IDLE_ACTIVITY }, NOW)).toBeNull();
        expect(workingOperation({}, NOW)).toBeNull();
    });

    it("the oldest tool call that has run for 3 s, by its command", () => {
        const op = workingOperation({ activity: withTools(tool()) }, NOW)!;
        expect(op.bucket.id).toBe("build");
        expect(op.since).toBe(NOW - 252_000);
        expect(workingOperationText(op, NOW)).toBe("Building 4m 12s");
    });

    it("a call younger than 3 s doesn't count, so quick calls don't flicker", () => {
        expect(workingOperation({ activity: withTools(tool({ startedAt: NOW - 2_999 })) }, NOW)).toBeNull();
        expect(workingOperation({ activity: withTools(tool({ startedAt: NOW - 3_000 })) }, NOW)?.bucket.id).toBe("build");
    });

    it("skips a young call for the older one behind it, and a subagent's own calls", () => {
        const young = tool({ id: "t0", startedAt: NOW - 500 });
        const own = tool({ id: "t2", parentId: "agent-call", command: "cargo test" });
        expect(workingOperation({ activity: withTools(young, own, tool({ id: "t3", command: "vitest run" })) }, NOW)?.bucket.id).toBe("test");
        expect(workingOperation({ activity: withTools(own) }, NOW)).toBeNull();
    });

    it("a tool with no name or command is the fallback", () => {
        expect(workingOperation({ activity: withTools(tool({ name: undefined, command: undefined })) }, NOW)?.bucket.id).toBe("other");
    });

    it("an Agent call is delegated work, a search tool is a search", () => {
        expect(workingOperation({ activity: withTools(tool({ name: "Agent", command: undefined })) }, NOW)?.bucket.id).toBe("delegate");
        expect(workingOperation({ activity: withTools(tool({ name: "Grep", command: undefined })) }, NOW)?.bucket.id).toBe("search");
    });

    it("what the turn is waiting on outranks a running tool: the person, a rate limit, a compaction", () => {
        const activity = withTools(tool());
        expect(workingOperation({ activity, needsYou: "Waiting for your answer" }, NOW)?.bucket.id).toBe("wait");
        expect(workingOperation({ activity, waitingReason: "rate_limited" }, NOW)?.bucket.id).toBe("hold");
        const compacting = { trigger: "auto" as const, startedAt: NOW - 61_000 };
        const op = workingOperation({ activity, compacting }, NOW)!;
        expect(op.bucket.id).toBe("compact");
        expect(workingOperationText(op, NOW)).toBe("Compacting 1m 1s");
    });

    it("says only the word when the start isn't known", () => {
        expect(workingOperationText(workingOperation({ needsYou: "x" }, NOW)!, NOW)).toBe("Waiting");
    });
});
