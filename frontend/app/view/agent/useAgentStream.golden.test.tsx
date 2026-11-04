// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentStream, pinned as it is: a Claude Code stream-json session goes in through the transcript
 * subject and everything the hook does with it comes out as one ordered log — each dispatch to the
 * pane and document reducers, each RPC it makes, and the document the pane ends up with. The log is
 * compared with a committed copy (useAgentStream.golden.json), so a refactor of the hook that changes
 * what it does fails here. A deliberate behaviour change regenerates the copy:
 * UPDATE_GOLDEN=1 npx vitest run frontend/app/view/agent/useAgentStream.golden.test.tsx
 * The frames are shaped like the CLI's; their content is made up.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type FileListener = (msg: { fileop: string; data64?: string }) => void;
const listeners: FileListener[] = [];
const rpcLog: string[] = [];
vi.mock("@/app/store/mps", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/mps")>()),
    getFileSubject: () => ({
        subscribe: (fn: FileListener) => {
            listeners.push(fn);
            return { unsubscribe: () => listeners.splice(listeners.indexOf(fn), 1) };
        },
    }),
}));
vi.mock("@/app/store/rpc-api", () => {
    const nothing = new Proxy({}, { get: (_t, name) => async (_c: unknown, data: unknown) => (rpcLog.push(`${String(name)} ${JSON.stringify(data ?? null)}`), []) });
    return { RpcApi: nothing };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { registerPane, unregisterPane } from "@/app/store/agent-pane-registration";
import { useAgentStream } from "./useAgentStream";

const BLOCK = "stream-golden";
const GOLDEN = join(process.cwd(), "frontend/app/view/agent/useAgentStream.golden.json");

/** One transcript append: `text` as srv writes it, a line each. */
function feedText(text: string): void {
    const data64 = Buffer.from(text + "\n").toString("base64");
    for (const fn of [...listeners]) fn({ fileop: "append", data64 });
}
const feed = (frame: object) => feedText(JSON.stringify(frame));

const ev = (event: object) => ({ type: "stream_event", event });
const messageStart = (id: string, input: number) =>
    ev({ type: "message_start", message: { id, model: "claude-test", usage: { input_tokens: input, cache_read_input_tokens: 2_000 } } });
const blockStart = (index: number, content_block: object) => ev({ type: "content_block_start", index, content_block });
const delta = (index: number, d: object) => ev({ type: "content_block_delta", index, delta: d });
const blockStop = (index: number) => ev({ type: "content_block_stop", index });
const toolResult = (id: string) => ({ type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: id, content: "ok" }] } });

/** The session: noise, a turn with thinking, text and a tool, a compaction, a fresh-session outcome, the turn's end, a truncate. */
const SESSION: Array<() => void> = [
    () => feedText("npm warn something printed by the CLI"),
    () => feedText(""),
    () => feedText('{"type": "oops'),
    () => feed({ type: "stderr", text: "disk almost full" }),
    () => feed({ type: "stderr", text: "[WARN] deprecated flag" }),
    () => feed({ type: "system", subtype: "init", session_id: "s-1", model: "claude-test" }),
    () => feed({ type: "system", subtype: "task_notification", task_id: "bg1", status: "completed", summary: "build done" }),
    () => feed(messageStart("m1", 10)),
    () => feed(blockStart(0, { type: "thinking", thinking: "" })),
    () => feed(delta(0, { type: "thinking_delta", thinking: "Weighing the options. " })),
    () => feed(blockStop(0)),
    () => feed(blockStart(1, { type: "text", text: "" })),
    () => feed(delta(1, { type: "text_delta", text: "Let me look at the file." })),
    () => feed(blockStop(1)),
    () => feed(blockStart(2, { type: "tool_use", id: "t1", name: "Read", input: {} })),
    () => feed(delta(2, { type: "input_json_delta", partial_json: JSON.stringify({ file_path: "/work/src/a.ts" }) })),
    () => feed(blockStop(2)),
    // The same call again, from its assistant frame: counted once.
    () => feed({ type: "assistant", message: { id: "m1", model: "claude-test", usage: { input_tokens: 10, cache_read_input_tokens: 2_000 }, content: [] } }),
    () => feed(toolResult("t1")),
    () => feed(ev({ type: "message_delta", usage: { output_tokens: 57 } })),
    () => feed({ type: "system", subtype: "compact_boundary", uuid: "u-1", timestamp: "2026-11-03T10:00:00Z", compact_metadata: { trigger: "manual", pre_tokens: 90_000, post_tokens: 4_000, duration_ms: 5_000 } }),
    () => feed({ type: "system", subtype: "compact_boundary", uuid: "u-2", compact_metadata: { trigger: "auto" } }),
    // The first call after the boundary fills the card with the context's real size.
    () => feed(messageStart("m2", 6_000)),
    () => feed(blockStart(0, { type: "text", text: "" })),
    () => feed(delta(0, { type: "text_delta", text: "Done, here is where I am." })),
    () => feed(blockStop(0)),
    () => feed({ type: "system", subtype: "agentmux_session_outcome", outcome: "resumed", attempted_sid: "s-1", actual_sid: "s-1", timestamp: "2026-11-03T10:01:00Z" }),
    () => feed({ type: "system", subtype: "agentmux_session_outcome", outcome: "fresh", attempted_sid: "s-1", actual_sid: "s-2", continued: true, idle_start: false, timestamp: "2026-11-03T10:02:00Z" }),
    () => feed({ type: "result", subtype: "success", duration_ms: 12_000, num_turns: 2 }),
    () => feed(messageStart("m3", 8_000)),
    () => feed(ev({ type: "message_delta", usage: { output_tokens: 12 } })),
    () => feed({ type: "result", subtype: "success", duration_ms: 3_000, num_turns: 1 }),
    () => { for (const fn of [...listeners]) fn({ fileop: "truncate" }); },
    () => feed(messageStart("m4", 500)),
    () => feed({ type: "result", subtype: "success", duration_ms: 1_000, num_turns: 1 }),
];

/** Run the whole session through the hook and return the ordered log. */
function run(): unknown {
    const log: string[] = [];
    const model = registerPane(BLOCK, { agentId: "golden" });
    for (const name of ["dispatchPane", "dispatchDoc"] as const) {
        const real = model[name].bind(model) as (cmd: { type: string }, ...rest: unknown[]) => unknown;
        (model as unknown as Record<string, unknown>)[name] = (cmd: { type: string }, ...rest: unknown[]) => {
            // A flush stamps itself with the animation frame's own clock, which the fake timers do not own.
            const shown = cmd.type === "StreamFlushObserved" ? { ...cmd, at: "<frame clock>" } : cmd;
            log.push(`${name === "dispatchPane" ? "pane" : "doc"} ${JSON.stringify(shown)}`);
            return real(cmd, ...rest);
        };
    }
    let stream: ReturnType<typeof useAgentStream> | undefined;
    const { unmount } = render(() => {
        stream = useAgentStream({
            blockId: BLOCK,
            model,
            outputFormat: "claude-stream-json",
            documentNodes: model.document,
            turnPhase: () => model.state.turnPhase,
            compacting: () => model.state.compacting,
            enabled: true,
            provider: "claude",
            agentName: "golden",
        });
        return <div />;
    });
    for (const step of SESSION) {
        step();
        vi.advanceTimersByTime(50); // a frame: the flush queue lands what the step brought
        log.push(`-- after step: phase=${model.state.turnPhase.kind}`);
    }
    vi.advanceTimersByTime(10_000);
    const document = JSON.parse(JSON.stringify(model.document()));
    const registryTasks = stream!().length;
    unmount();
    vi.advanceTimersByTime(100);
    return { log, rpc: [...rpcLog], document, registryTasks, afterUnmount: { phase: model.state.turnPhase.kind } };
}

describe("useAgentStream, pinned", () => {
    beforeEach(() => {
        rpcLog.length = 0;
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date", "requestAnimationFrame", "cancelAnimationFrame"] });
        vi.setSystemTime(new Date("2026-11-03T10:00:00Z"));
        vi.spyOn(Math, "random").mockReturnValue(0.123456789);
    });
    afterEach(() => {
        cleanup();
        unregisterPane(BLOCK);
        listeners.length = 0;
        vi.useRealTimers();
        vi.restoreAllMocks();
    });

    it("does with a session of frames exactly what the committed log says", () => {
        const got = JSON.parse(JSON.stringify(run()));
        if (process.env.UPDATE_GOLDEN === "1" || !existsSync(GOLDEN)) {
            writeFileSync(GOLDEN, JSON.stringify(got, null, 1) + "\n");
        }
        expect(got).toEqual(JSON.parse(readFileSync(GOLDEN, "utf8")));
    });

    it("sees the frames the session is made of (so the log is not empty by accident)", () => {
        const got = run() as { log: string[] };
        const text = got.log.join("\n");
        for (const wanted of ["pane {\"type\":\"StreamSubscribe\"", "\"type\":\"TokensIn\"", "\"type\":\"TokensOut\"", "\"type\":\"CompactionBoundary\"", "\"type\":\"ContextInvalidated\"", "\"type\":\"TurnReset\"", "\"type\":\"ToolStart\"", "\"type\":\"ToolEnd\""]) {
            expect(text).toContain(wanted);
        }
        // The truncate at the end empties the document, so the nodes are looked for in the flushes that landed them.
        for (const kind of ["markdown", "tool", "context_compacted", "session_outcome"]) expect(text).toContain(`"type":"${kind}"`);
    });
});
