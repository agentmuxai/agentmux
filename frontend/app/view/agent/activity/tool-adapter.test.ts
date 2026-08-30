// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { DocumentNode, ToolNode } from "../types";
import { hasRunningPromotedTool, nextToolPromotionAt, TOOL_PROMOTION_MS, toolActivities, toolToActivity } from "./tool-adapter";

function mkBash(overrides: Partial<ToolNode> = {}): ToolNode {
    return {
        type: "tool",
        id: "tool-1",
        tool: "Bash",
        status: "running",
        // NOT a sleep, deliberately. `sleep-detect.ts` promotes a
        // whole-command sleep immediately, bypassing TOOL_PROMOTION_MS — so a
        // `sleep` fixture here would make every threshold assertion below
        // vacuously pass. This must stay a command that only the duration rule
        // can catch.
        params: { command: "cargo test -p agentmux-srv" },
        collapsed: false,
        summary: "",
        timestamp: 0,
        ...overrides,
    };
}

describe("toolToActivity", () => {
    it("maps a running Bash node to a running, non-stoppable activity", () => {
        const n = mkBash({ id: "t1", timestamp: 1000, params: { command: "cargo test -p agentmux-srv" } });
        const a = toolToActivity(n);
        expect(a.id).toBe("t1");
        expect(a.kind).toBe("tool");
        expect(a.status).toBe("running");
        expect(a.startedAt).toBe(1000);
        expect(a.endedAt).toBeUndefined();
        expect(a.canStop).toBe(false);
        expect(a.title).toBe("cargo test -p agentmux-srv");
        expect(a.tool).toBe(n);
    });

    it("falls back to the tool name when params carry no command text", () => {
        const n = mkBash({ params: {}, toolName: "Bash" });
        expect(toolToActivity(n).title).toBe("Bash");
    });

    it("maps a finished call to done/error with endedAt = timestamp + duration", () => {
        const done = toolToActivity(mkBash({ status: "success", timestamp: 1000, duration: 45 }));
        expect(done.status).toBe("done");
        expect(done.endedAt).toBe(1000 + 45_000);

        const err = toolToActivity(mkBash({ status: "failed", timestamp: 1000, duration: 45 }));
        expect(err.status).toBe("error");
        expect(err.endedAt).toBe(1000 + 45_000);
    });

    it("maps denied/canceled to stopped — cut off, not a failure signal", () => {
        expect(toolToActivity(mkBash({ status: "denied" })).status).toBe("stopped");
        expect(toolToActivity(mkBash({ status: "canceled" })).status).toBe("stopped");
    });
});

describe("toolActivities", () => {
    it("excludes a running Bash call that hasn't crossed the promotion threshold yet", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: 1000 })];
        expect(toolActivities(nodes, 1000 + TOOL_PROMOTION_MS - 1)).toEqual([]);
    });

    it("includes a running Bash call exactly at and past the promotion threshold", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: 1000 })];
        expect(toolActivities(nodes, 1000 + TOOL_PROMOTION_MS).map((a) => a.id)).toEqual(["t1"]);
        expect(toolActivities(nodes, 1000 + TOOL_PROMOTION_MS + 5000).map((a) => a.id)).toEqual(["t1"]);
    });

    it("never promotes a non-Bash tool call, regardless of duration", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", tool: "Read", timestamp: 0 })];
        expect(toolActivities(nodes, 1_000_000)).toEqual([]);
    });

    it("ignores nodes with no timestamp (pre-field-add back-compat)", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: undefined })];
        expect(toolActivities(nodes, 1_000_000)).toEqual([]);
    });

    it("keeps a finished call whose total duration crossed the threshold — inherits the dock's normal retention lifecycle instead of vanishing on ToolEnd", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", status: "success", timestamp: 1000, duration: 40 })];
        // Long after it finished — still returned; ActivityDock's own
        // RETENTION_MS + endedAt filtering, not this function, decides
        // when the row actually disappears from view.
        const result = toolActivities(nodes, 1000 + 40_000 + 60_000);
        expect(result.map((a) => a.id)).toEqual(["t1"]);
        expect(result[0].status).toBe("done");
    });

    it("never promotes a call that finished before crossing the threshold", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", status: "success", timestamp: 1000, duration: 5 })];
        expect(toolActivities(nodes, 1000 + 5000)).toEqual([]);
    });
});

describe("nextToolPromotionAt", () => {
    it("returns null when nothing is pending promotion", () => {
        expect(nextToolPromotionAt([], 0)).toBeNull();
    });

    it("returns the promotion instant of a still-running, not-yet-promoted call", () => {
        const nodes: DocumentNode[] = [mkBash({ timestamp: 1000 })];
        expect(nextToolPromotionAt(nodes, 1000)).toBe(1000 + TOOL_PROMOTION_MS);
    });

    it("returns null once the call has already crossed the threshold (nothing left to schedule)", () => {
        const nodes: DocumentNode[] = [mkBash({ timestamp: 1000 })];
        expect(nextToolPromotionAt(nodes, 1000 + TOOL_PROMOTION_MS)).toBeNull();
    });

    it("picks the earliest pending promotion among several running calls", () => {
        const nodes: DocumentNode[] = [
            mkBash({ id: "t1", timestamp: 5000 }),
            mkBash({ id: "t2", timestamp: 1000 }),
        ];
        expect(nextToolPromotionAt(nodes, 1000)).toBe(1000 + TOOL_PROMOTION_MS);
    });

    it("ignores a finished call — nothing left to schedule for it", () => {
        const nodes: DocumentNode[] = [mkBash({ status: "success", timestamp: 1000, duration: 40 })];
        expect(nextToolPromotionAt(nodes, 2000)).toBeNull();
    });

    it("moves on to the next-earliest pending call once the first is excluded (simulates a reschedule after the first fires)", () => {
        const nodes: DocumentNode[] = [
            mkBash({ id: "t1", timestamp: 1000 }),
            mkBash({ id: "t2", timestamp: 5000 }),
        ];
        // At the instant t1 crosses the threshold, a re-run should now
        // surface t2's own promotion instant instead of returning null.
        expect(nextToolPromotionAt(nodes, 1000 + TOOL_PROMOTION_MS)).toBe(5000 + TOOL_PROMOTION_MS);
    });
});

describe("hasRunningPromotedTool", () => {
    it("is false before the threshold and true at/after it, for a running call", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: 1000 })];
        expect(hasRunningPromotedTool(nodes, 1000 + TOOL_PROMOTION_MS - 1)).toBe(false);
        expect(hasRunningPromotedTool(nodes, 1000 + TOOL_PROMOTION_MS)).toBe(true);
    });

    it("is false for a finished call still lingering in the dock's retention window — must not suppress a different, newly-started tool's working-row text", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", status: "success", timestamp: 1000, duration: 40 })];
        expect(hasRunningPromotedTool(nodes, 1000 + 40_000 + 1000)).toBe(false);
    });

    it("is false with no nodes", () => {
        expect(hasRunningPromotedTool([], 1_000_000)).toBe(false);
    });
});

// ── Backgrounded calls (issue #2490) ──────────────────────────────────

function mkBgBash(overrides: Partial<ToolNode> = {}): ToolNode {
    return mkBash({
        id: "toolu_bg1",
        status: "success",
        params: { command: "task dev", run_in_background: true },
        timestamp: 1000,
        duration: 0.4,
        result: { stdout: "Command running in background with ID: b12345. Output is being written to: …", stderr: "", exitCode: 0 },
        ...overrides,
    });
}

function mkNotification(toolUseId: string, status: string, timestamp = 500_000): DocumentNode {
    return {
        type: "user_message",
        id: `user-${toolUseId}`,
        message:
            `<task-notification>\n<task-id>b12345</task-id>\n<tool-use-id>${toolUseId}</tool-use-id>\n` +
            `<output-file>C:\tmp\b12345.output</output-file>\n<status>${status}</status>\n` +
            `<summary>Background command finished</summary>\n</task-notification>`,
        timestamp,
    };
}

describe("toolActivities — backgrounded calls", () => {
    it("shows an accepted background launch as running immediately, no 30s threshold", () => {
        const nodes: DocumentNode[] = [mkBgBash()];
        // Well under the promotion threshold — an ordinary sub-second call
        // would be invisible; a declared background task must not be.
        const acts = toolActivities(nodes, 2000);
        expect(acts).toHaveLength(1);
        expect(acts[0].status).toBe("running");
        expect(acts[0].startedAt).toBe(1000);
        expect(acts[0].endedAt).toBeUndefined();
        expect(acts[0].title).toBe("task dev");
    });

    it("stays running until ITS OWN task-notification lands — another call's doesn't end it", () => {
        const nodes: DocumentNode[] = [mkBgBash({ id: "toolu_a" }), mkNotification("toolu_other", "completed")];
        const acts = toolActivities(nodes, 2000);
        expect(acts).toHaveLength(1);
        expect(acts[0].status).toBe("running");
    });

    it("ends as done/error per the notification's status, endedAt = notification time", () => {
        const completed = toolActivities(
            [mkBgBash({ id: "toolu_ok" }), mkNotification("toolu_ok", "completed", 600_000)],
            700_000,
        );
        expect(completed[0].status).toBe("done");
        expect(completed[0].endedAt).toBe(600_000);

        const failed = toolActivities(
            [mkBgBash({ id: "toolu_bad" }), mkNotification("toolu_bad", "failed", 600_000)],
            700_000,
        );
        expect(failed[0].status).toBe("error");
    });

    it("ends as stopped on a notification with an unrecognized status — never running forever", () => {
        const acts = toolActivities(
            [mkBgBash({ id: "toolu_x" }), mkNotification("toolu_x", "killed")],
            700_000,
        );
        expect(acts[0].status).toBe("stopped");
    });

    it("a REFUSED background launch is not a live task — ordinary duration rules apply", () => {
        // status failed = the harness rejected the command; sub-second, so
        // the threshold gate drops it entirely.
        const nodes: DocumentNode[] = [mkBgBash({ status: "failed" })];
        expect(toolActivities(nodes, 2000)).toEqual([]);
    });

    it("issue #2518: a call that asked for run_in_background but finished too fast to actually detach is NOT treated as a live background task", () => {
        // The harness itself decides per-call whether a command actually gets
        // detached — a command that finishes fast enough is returned
        // synchronously with the command's real output (the same
        // `<exited N in Ts>` shape an ordinary call gets), not the "Command
        // running in background with ID: …" acceptance message, even though
        // run_in_background: true was passed. Without the result-content
        // check, this was misclassified as "launch accepted, wait for a
        // <task-notification> that will never come" — a dock row stuck
        // `running` forever. Confirmed live: 11 of 17 backgrounded calls in
        // a single session got stuck exactly this way.
        const fastFinish = mkBgBash({
            duration: 13.38,
            result: { stdout: "<exited 0 in 13.38s>\n707M    /c/Users/area54/.cargo", stderr: "", exitCode: 0 },
        });
        // Long after it finished — must resolve via the ordinary
        // duration/threshold rules, not the "wait for notification" path.
        expect(toolActivities([fastFinish], 1000 + 13_380 + 60_000)).toEqual([]);
    });

    it("codex P1 on PR #2519: a genuine background acceptance arriving via the result.content fallback shape (not stdout) is still recognized", () => {
        // claude-translator.ts's buildToolResults falls back to a plain
        // `{ content: string }` shape (instead of the structured
        // `{ stdout, stderr, interrupted }` sibling) when Claude omits a
        // terminal-shaped tool_use_result or returns multiple tool_result
        // blocks. A stdout-only check would reject this as "not accepted"
        // and drop a still-running background launch from the dock
        // entirely — worse than the original bug, since a stuck-forever
        // entry is at least visible.
        const nodes: DocumentNode[] = [
            mkBgBash({
                result: { content: "Command running in background with ID: b12345. Output is being written to: …" } as any,
            }),
        ];
        const acts = toolActivities(nodes, 2000);
        expect(acts).toHaveLength(1);
        expect(acts[0].status).toBe("running");
    });

    it("a foreground call is untouched by an unrelated notification in the document", () => {
        const nodes: DocumentNode[] = [
            mkBash({ id: "fg", timestamp: 1000 }),
            mkNotification("toolu_other", "completed"),
        ];
        const acts = toolActivities(nodes, 1000 + TOOL_PROMOTION_MS);
        expect(acts).toHaveLength(1);
        expect(acts[0].id).toBe("fg");
        expect(acts[0].status).toBe("running");
    });
});

describe("whole-command sleeps promote immediately (sleep-detect.ts)", () => {
    const sleepNode = (over: Partial<ToolNode> = {}) =>
        mkBash({ id: "s1", timestamp: 1000, params: { command: "sleep 300" }, ...over });

    /** The point of the feature: a self-declared wait needs no detection
     *  window. Measured median for these in real transcripts is 61s, so the
     *  old behaviour spent the first 30s of every one saying "Working…". */
    it("docks a bare sleep at t=0, without waiting for TOOL_PROMOTION_MS", () => {
        const nodes: DocumentNode[] = [sleepNode()];
        expect(toolActivities(nodes, 1000).map((a) => a.id)).toEqual(["s1"]);
        expect(toolActivities(nodes, 1001).map((a) => a.id)).toEqual(["s1"]);
    });

    it("carries sleepMs so the row can render a real countdown", () => {
        expect(toolActivities([sleepNode()], 1000)[0].sleepMs).toBe(300_000);
    });

    it("suppresses the working row's tool text immediately too", () => {
        // Otherwise the dock would show the sleep while AgentWorkingRow went on
        // repeating it for another 30s.
        expect(hasRunningPromotedTool([sleepNode()], 1000)).toBe(true);
    });

    it("schedules no promotion timer — it is already promoted", () => {
        expect(nextToolPromotionAt([sleepNode()], 1000)).toBeNull();
    });

    /** It was docked from t=0, so it must RESOLVE in place (done, then normal
     *  retention) rather than vanish the instant it ends — which is what the
     *  duration rule alone would do for anything under the threshold. */
    it("keeps a short sleep's row after it finishes, instead of dropping it", () => {
        const finished = sleepNode({ params: { command: "sleep 12" }, status: "success", duration: 12 });
        const acts = toolActivities([finished], 1000 + 12_000 + 1_000);
        expect(acts).toHaveLength(1);
        expect(acts[0].status).toBe("done");
        expect(acts[0].endedAt).toBe(1000 + 12_000);
    });

    /** The 76%-wrong case from the real-transcript measurement. A compound
     *  sleep must follow the ordinary duration rule — it is not a pure wait,
     *  and its total runtime is unknowable from the text. */
    it("does NOT fast-path a compound sleep, and gives it no countdown", () => {
        const compound = mkBash({ id: "c1", timestamp: 1000, params: { command: "sleep 90; tail -30 /tmp/b.log" } });
        expect(toolActivities([compound], 1000)).toEqual([]);
        expect(nextToolPromotionAt([compound], 1000)).toBe(1000 + TOOL_PROMOTION_MS);
        const promoted = toolActivities([compound], 1000 + TOOL_PROMOTION_MS);
        expect(promoted.map((a) => a.id)).toEqual(["c1"]);
        expect(promoted[0].sleepMs).toBeUndefined();
    });

    /** A micro-delay is below sleep-detect's floor, so it stays on the ordinary
     *  path and never takes a dock row at all. */
    it("ignores a micro-delay sleep entirely", () => {
        const micro = mkBash({ id: "m1", timestamp: 1000, params: { command: "sleep 2" }, status: "success", duration: 2 });
        expect(toolActivities([micro], 1000 + 60_000)).toEqual([]);
    });
});
