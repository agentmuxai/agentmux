// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Working row, end to end: Claude Code stream-json lines go in through
 * the transcript subject, and the real useAgentStream, translator, pane model,
 * reducer, presenter and AgentWorkingRow carry them to what the user reads.
 * Only the transcript feed and the backend are fakes.
 *
 * It exists because two bugs passed every test of the pieces and were caught
 * by eye in a live build (SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md
 * §6.10): the row re-typed the same line every second, and Claude Code's
 * usage-report `rate_limit_event` ("allowed") showed "Rate limited — retrying".
 * The stream below is shaped like a real turn; its content is made up.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// ── The transcript feed: the one way lines reach the hook ───────────────────
type FileListener = (msg: { fileop: string; data64?: string }) => void;
const listeners: FileListener[] = [];
vi.mock("@/app/store/mps", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/mps")>()),
    getFileSubject: () => ({
        subscribe: (fn: FileListener) => {
            listeners.push(fn);
            return { unsubscribe: () => listeners.splice(listeners.indexOf(fn), 1) };
        },
    }),
}));
// ── The backend: every RPC answers with nothing ─────────────────────────────
vi.mock("@/app/store/rpc-api", () => {
    // An empty list: reads as "no tasks" to a caller that wants a list, and
    // as a response with no fields to one that wants an object.
    const nothing = new Proxy({}, { get: () => async () => [] });
    return { RpcApi: nothing };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { registerPane, unregisterPane, type AgentPaneModel } from "@/app/store/agent-pane-registration";
import { AgentWorkingRow } from "./components/AgentFooter";
import { useAgentStream } from "./useAgentStream";

const BLOCK = "stream-replay";

/** Feed one stream-json line, as srv appends it to the transcript. */
function feed(frame: object): void {
    const data64 = Buffer.from(JSON.stringify(frame) + "\n").toString("base64");
    for (const fn of [...listeners]) fn({ fileop: "append", data64 });
}

// ── Claude Code stream-json frames (shapes as the CLI writes them) ──────────
const usageReport = (status: string) => ({
    type: "rate_limit_event",
    rate_limit_info: { status, resetsAt: 1_900_000_000, rateLimitType: "five_hour" },
});
const messageStart = (id: string) => ({
    type: "stream_event",
    event: { type: "message_start", message: { id, model: "claude-test", usage: { input_tokens: 10, cache_read_input_tokens: 2_000 } } },
});
const blockStart = (index: number, content_block: object) => ({ type: "stream_event", event: { type: "content_block_start", index, content_block } });
const delta = (index: number, d: object) => ({ type: "stream_event", event: { type: "content_block_delta", index, delta: d } });
const blockStop = (index: number) => ({ type: "stream_event", event: { type: "content_block_stop", index } });
const toolResult = (id: string) => ({
    type: "user",
    message: { role: "user", content: [{ type: "tool_result", tool_use_id: id, content: "ok" }] },
});

/** One tool call, streamed: its block, its input, its stop. */
function toolCall(at: number, index: number, id: string, name: string, input: object): Array<[number, object]> {
    return [
        [at, blockStart(index, { type: "tool_use", id, name, input: {} })],
        [at + 50, delta(index, { type: "input_json_delta", partial_json: JSON.stringify(input) })],
        [at + 100, blockStop(index)],
    ];
}

/** A turn: think, read a file quickly, run the tests for 20 s, reply. Claude
 *  Code reports usage (`rate_limit_event`, "allowed") at each call. */
function turn(report: string): Array<[number, object]> {
    const frames: Array<[number, object]> = [
        [100, messageStart("m1")],
        [150, usageReport(report)],
        [300, blockStart(0, { type: "thinking", thinking: "" })],
    ];
    for (let t = 400; t <= 4_400; t += 500) frames.push([t, delta(0, { type: "thinking_delta", thinking: "Weighing the next step. " })]);
    frames.push([4_500, blockStop(0)]);
    frames.push(...toolCall(4_600, 1, "t1", "Read", { file_path: "/work/src/a.ts" }));
    frames.push([5_000, toolResult("t1")]);
    frames.push([5_300, messageStart("m2")], [5_350, usageReport(report)]);
    frames.push(...toolCall(5_500, 0, "t2", "Bash", { command: "npm test", description: "Run the unit tests" }));
    frames.push([25_000, toolResult("t2")]);
    frames.push([25_300, messageStart("m3")], [25_350, usageReport(report)], [25_400, blockStart(0, { type: "text", text: "" })]);
    for (let t = 25_500; t <= 28_000; t += 250) frames.push([t, delta(0, { type: "text_delta", text: "Done. " })]);
    frames.push([28_100, blockStop(0)], [28_500, { type: "result", subtype: "success", duration_ms: 28_400, num_turns: 3 }]);
    return frames;
}

/** Mount the hook and the row for one pane, the way the agent view does. */
function mount(): { container: HTMLElement; model: AgentPaneModel } {
    const model = registerPane(BLOCK, { agentId: "replay" });
    const { container } = render(() => {
        useAgentStream({
            blockId: BLOCK,
            model,
            outputFormat: "claude-stream-json",
            documentNodes: model.document,
            turnPhase: () => model.state.turnPhase,
            compacting: () => model.state.compacting,
            enabled: true,
            provider: "claude",
            agentName: "replay",
        });
        const streaming = () => (model.state.turnPhase.kind === "Streaming" ? model.state.turnPhase : null);
        return (
            <AgentWorkingRow
                loading={true}
                activitySummary="Fix the login redirect loop"
                turnTokens={model.state.turnTokens}
                turnLedger={model.state.turnLedger}
                activity={model.state.activity}
                waitingReason={streaming()?.waitingReason ?? null}
                retryAfterMs={streaming()?.retryAfterMs ?? null}
                compacting={model.state.compacting}
                reconnecting={model.state.reconnecting}
            />
        );
    });
    return { container, model };
}

/** Play `frames` on the fake clock, reading the row's line every `sampleMs`. */
function play(container: HTMLElement, frames: Array<[number, object]>, endMs: number, sampleMs = 50): string[] {
    const sorted = [...frames].sort((a, b) => a[0] - b[0]);
    const seen: string[] = [];
    let next = 0;
    for (let t = 0; t <= endMs; t += sampleMs) {
        while (next < sorted.length && sorted[next][0] <= t) feed(sorted[next++][1]);
        vi.advanceTimersByTime(sampleMs);
        seen.push(container.querySelector(".agent-working-row-primary")?.textContent ?? "");
    }
    return seen;
}

/** Each place a line that was fully on screen was wiped and typed again. */
function retypes(seen: string[]): string[] {
    const out: string[] = [];
    let full = "";
    for (let i = 1; i < seen.length; i++) {
        const prev = seen[i - 1];
        const cur = seen[i];
        if (cur.length >= prev.length) {
            if (cur === prev) full = cur;
            continue;
        }
        // It got shorter. Fine if a different line is typing in; a re-type
        // if it is the start of the line that was already whole.
        if (full && full.startsWith(cur) && seen.slice(i).some((s) => s === full)) out.push(`"${full}" re-typed at sample ${i}`);
    }
    return [...new Set(out)];
}

describe("the Working row, from Claude Code's stream to the screen", () => {
    beforeEach(() => {
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date", "requestAnimationFrame", "cancelAnimationFrame"] });
    });
    afterEach(() => {
        cleanup();
        unregisterPane(BLOCK);
        listeners.length = 0;
        vi.useRealTimers();
    });

    it("a turn with Claude Code's usage reports: no false rate limit, no line typed twice, the test run named", () => {
        const { container } = mount();
        const seen = play(container, turn("allowed"), 30_000);
        expect(seen.filter((s) => s.startsWith("Rate limited"))).toEqual([]);
        expect(retypes(seen)).toEqual([]);
        expect(seen.some((s) => s.startsWith("Running the unit tests"))).toBe(true);
        expect(seen.some((s) => s.startsWith("Thinking"))).toBe(true);
    });

    it("a rejected rate_limit_event is a rate limit, and says so while the CLI waits", () => {
        const { container } = mount();
        // Held back: the call opens, Claude Code reports the rejection, and
        // nothing more arrives until it retries.
        const seen = play(container, [[100, messageStart("m1")], [150, usageReport("rejected")]], 6_000);
        expect(seen.some((s) => s.startsWith("Rate limited"))).toBe(true);
    });
});
