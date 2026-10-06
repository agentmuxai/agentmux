// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { HistoryParser, parseHistoryLines } from "./parseHistoryLines";
import { contextCompactedNodeId } from "./compact-boundary";
import { createMemoryReinjectionController } from "./memory-reinjection-controller";
import { update } from "@/app/store/agent-pane-state/reducer";
import { initialState } from "@/app/store/agent-pane-state/types";
import { composeReinjectionMessage } from "./memory-reinjection";
import type { ToolNode } from "./types";

vi.mock("./memory-reinjection-controller", async (importOriginal) => {
    const actual = await importOriginal<typeof import("./memory-reinjection-controller")>();
    return { ...actual, createMemoryReinjectionController: vi.fn(actual.createMemoryReinjectionController) };
});

// The Claude translator passes through events that already match the
// StreamEvent shape (`if (this.isStreamEvent(rawEvent))` branch), so
// each test line is JSON-stringified StreamEvent.
const line = (event: object): string => JSON.stringify(event);

type Usage = { input_tokens: number; cache_creation_input_tokens?: number; cache_read_input_tokens?: number };
/** A main-agent API call's `message_start`, as Claude Code streams it. */
const messageStart = (id: string, usage: Usage, model = "claude-sonnet-5-5"): string =>
    JSON.stringify({
        type: "stream_event",
        event: { type: "message_start", message: { id, model, role: "assistant", content: [], usage } },
        parent_tool_use_id: null,
    });
/** The same call's `assistant` frame (no content, so it adds no node). */
const assistantCall = (id: string, usage: Usage, model = "claude-sonnet-5-5"): string =>
    JSON.stringify({
        type: "assistant",
        message: { id, model, role: "assistant", content: [], usage: { ...usage, output_tokens: 4 } },
        parent_tool_use_id: null,
    });
/** A turn's `result`: usage summed over its calls, and the window CLI 2.1.288 reports. */
const resultFrame = (summedInput: number): string =>
    JSON.stringify({
        type: "result",
        subtype: "success",
        is_error: false,
        num_turns: 3,
        result: "ok",
        usage: { input_tokens: 6, cache_creation_input_tokens: 0, cache_read_input_tokens: summedInput - 6, output_tokens: 36 },
        modelUsage: { "claude-sonnet-5-5": { contextWindow: 1_000_000, maxOutputTokens: 128_000 } },
    });
const freshOutcome = (): string =>
    JSON.stringify({
        type: "system",
        subtype: "agentmux_session_outcome",
        outcome: "fresh",
        attempted_sid: "sid-1",
        actual_sid: null,
        timestamp: "2026-08-10T08:00:00Z",
    });

describe("parseHistoryLines", () => {
    it("merges same-id tool_call → tool_result so the tool ends success, not stuck running", () => {
        // Codex P1 on PR #1104: the previous "first-wins by id" rule
        // dropped the tool_result event during replay, leaving the
        // tool stuck at `status: "running"` on rendered history pages.
        // The orphan-scrub pass would then turn it into "canceled",
        // mislabeling a successfully-completed tool.
        const lines = [
            line({ type: "tool_call", tool: "Bash", id: "tool-1", params: { command: "ls" } }),
            line({ type: "tool_result", tool: "Bash", id: "tool-1", status: "success", duration: 0.1 }),
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(1);
        const tool = nodes[0] as ToolNode;
        expect(tool.id).toBe("tool-1");
        expect(tool.status).toBe("success");
    });

    it("preserves insertion order across same-id replacements", () => {
        // Tool replays in the middle of text shouldn't move the tool
        // to the end — its position is where its `tool_call` first
        // appeared in the stream.
        const lines = [
            line({ type: "text", content: "before" }),
            line({ type: "tool_call", tool: "Read", id: "tool-1", params: { file_path: "x" } }),
            line({ type: "text", content: "after" }),
            // tool_result lands later but should NOT push the tool past "after".
            line({ type: "tool_result", tool: "Read", id: "tool-1", status: "success", duration: 0 }),
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(3);
        expect(nodes[0].type).toBe("markdown");
        expect(nodes[1].type).toBe("tool");
        expect((nodes[1] as ToolNode).status).toBe("success");
        expect(nodes[2].type).toBe("markdown");
    });

    it("accumulates streaming thinking deltas in place (last delta = full text)", () => {
        // Streaming markdown / thinking deltas share an id; each event
        // carries the running accumulated content (parser appends and
        // emits a fresh object each time). With first-wins this would
        // freeze the rendered thought at "Let me " forever; with
        // last-wins, the final delta's full text is preserved.
        const lines = [
            line({ type: "thinking", content: "Let me " }),
            line({ type: "thinking", content: "think about this..." }),
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(1);
        const node = nodes[0] as any;
        expect(node.type).toBe("markdown");
        expect(node.content).toBe("Let me think about this...");
        expect(node.metadata.thinking).toBe(true);
    });

    it("skips corrupt and stderr lines silently", () => {
        const lines = [
            "{ not json",
            line({ type: "stderr", content: "ignore me" }),
            line({ type: "text", content: "real text" }),
            "",
            "   ",
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(1);
        expect((nodes[0] as any).content).toBe("real text");
    });

    it("a turn's session_end emits no node and is never read as the context size", () => {
        // A `result`'s usage sums every API call of the turn. Seeding the
        // context meter from it showed "17m" on a freshly opened pane
        // (REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md §2).
        const lines = [
            line({ type: "text", content: "hi" }),
            line({ type: "session_end", stats: { input_tokens: 17_000_000, output_tokens: 22 } }),
        ];
        const { nodes, lastContext } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(1);
        expect(lastContext).toBeNull();
    });

    it("returns a null lastContext when no main-agent call is present", () => {
        const lines = [line({ type: "text", content: "hi" })];
        const { lastContext, reportedContextWindows } = parseHistoryLines(lines, "claude-stream-json");
        expect(lastContext).toBeNull();
        expect(reportedContextWindows).toEqual({});
    });

    it("seeds from the main agent's LAST call — its whole prompt — not the turn's result total", () => {
        // The shape of a real three-call turn on CLI 2.1.288: each call's
        // prompt by message_start (and again by its assistant frame), then a
        // result whose usage is the three prompts summed (130,499).
        const lines = [
            messageStart("msg_1", { input_tokens: 2, cache_creation_input_tokens: 31_490, cache_read_input_tokens: 11_884 }),
            assistantCall("msg_1", { input_tokens: 2, cache_creation_input_tokens: 31_490, cache_read_input_tokens: 11_884 }),
            messageStart("msg_2", { input_tokens: 2, cache_creation_input_tokens: 124, cache_read_input_tokens: 43_374 }),
            messageStart("msg_3", { input_tokens: 2, cache_creation_input_tokens: 123, cache_read_input_tokens: 43_498 }),
            resultFrame(130_499),
        ];
        const stamps = lines.map((_, i) => 1_790_000_000_000 + i);
        const { lastContext, reportedContextWindows } = parseHistoryLines(lines, "claude-stream-json", undefined, stamps);
        expect(lastContext).toEqual({ tokens: 43_623, model: "claude-sonnet-5-5", at: 1_790_000_000_003 });
        expect(reportedContextWindows).toEqual({ "claude-sonnet-5-5": 1_000_000 });
    });

    it("reads a call from its assistant frame alone (a transcript without stream events)", () => {
        const lines = [assistantCall("msg_9", { input_tokens: 5, cache_creation_input_tokens: 0, cache_read_input_tokens: 80_000 })];
        expect(parseHistoryLines(lines, "claude-stream-json").lastContext).toEqual({
            tokens: 80_005,
            model: "claude-sonnet-5-5",
            at: null,
        });
    });

    it("ignores a subagent's calls: they have their own context and model", () => {
        const lines = [
            messageStart("msg_1", { input_tokens: 1, cache_read_input_tokens: 300_000 }),
            JSON.stringify({ ...JSON.parse(messageStart("msg_s", { input_tokens: 1, cache_read_input_tokens: 9_000 }, "claude-haiku-4-5")), parent_tool_use_id: "toolu_sub" }),
        ];
        expect(parseHistoryLines(lines, "claude-stream-json").lastContext).toMatchObject({
            tokens: 300_001,
            model: "claude-sonnet-5-5",
        });
    });

    it("ignores Claude Code's own zero-usage <synthetic> assistant frames", () => {
        const lines = [
            messageStart("msg_1", { input_tokens: 1, cache_read_input_tokens: 300_000 }),
            JSON.stringify({
                type: "assistant",
                message: { id: "msg_x", model: "<synthetic>", role: "assistant", content: [], usage: { input_tokens: 0, output_tokens: 0 } },
            }),
        ];
        expect(parseHistoryLines(lines, "claude-stream-json").lastContext?.tokens).toBe(300_001);
    });

    it("after a compaction boundary there is no context until the next call: post_tokens is not the size", () => {
        // CLI 2.1.288: post_tokens counts the summary messages only (1,417);
        // the next call's prompt, system prompt and tools included, was 39,490.
        const boundary = JSON.stringify({
            type: "system",
            subtype: "compact_boundary",
            compact_metadata: { trigger: "manual", pre_tokens: 40_697, post_tokens: 1_417, duration_ms: 6_448 },
            timestamp: "2026-09-24T10:00:00Z",
        });
        const upToBoundary = [messageStart("msg_1", { input_tokens: 1, cache_read_input_tokens: 40_672 }), boundary];
        expect(parseHistoryLines(upToBoundary, "claude-stream-json").lastContext).toBeNull();
        const andACall = [...upToBoundary, messageStart("msg_2", { input_tokens: 2, cache_read_input_tokens: 39_488 })];
        expect(parseHistoryLines(andACall, "claude-stream-json").lastContext?.tokens).toBe(39_490);
        // A boundary whose metadata doesn't parse is still a boundary.
        const unparsed = JSON.stringify({ type: "system", subtype: "compact_boundary" });
        expect(parseHistoryLines([messageStart("msg_1", { input_tokens: 5 }), unparsed], "claude-stream-json").lastContext).toBeNull();
    });

    it("reads no context from a provider whose stream isn't Claude Code's", () => {
        const lines = [messageStart("msg_1", { input_tokens: 1, cache_read_input_tokens: 40_000 })];
        expect(parseHistoryLines(lines, "gemini-json").lastContext).toBeNull();
        expect(parseHistoryLines(lines, "codex-json").lastContext).toBeNull();
    });

    it("resets the context at a fresh session boundary (codex P2 on PR #2507)", () => {
        // Window shape: [old call -> fresh boundary -> no new call].
        // The old session's usage must NOT hydrate the fresh session's
        // meter — the fresh model has none of those tokens.
        const lines = [
            messageStart("msg_1", { input_tokens: 1, cache_read_input_tokens: 400_000 }),
            line({ type: "text", content: "old turn" }),
            freshOutcome(),
            line({ type: "text", content: "new turn" }),
        ];
        expect(parseHistoryLines(lines, "claude-stream-json").lastContext).toBeNull();
    });

    it("post-boundary usage still hydrates after a fresh boundary", () => {
        const lines = [
            messageStart("msg_1", { input_tokens: 1, cache_read_input_tokens: 400_000 }),
            freshOutcome(),
            messageStart("msg_2", { input_tokens: 40, cache_read_input_tokens: 0 }),
        ];
        expect(parseHistoryLines(lines, "claude-stream-json").lastContext?.tokens).toBe(40);
    });

    // §4.4 of SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW
    // _2026_08_09.md: replayed nodes get receive-time stamps from the
    // output.tsidx sidecar (`stamps` parallel to `lines`); wire timestamps
    // win; 0/absent = unknown and must never stamp a node.
    describe("tsidx receive-time stamping (§4.4)", () => {
        it("stamps a timestamp-less replayed node from its line's batch time", () => {
            const lines = [
                line({ type: "text", content: "hello" }),
                line({ type: "tool_call", tool: "Bash", id: "tool-1", params: { command: "ls" } }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json", undefined, [1_700_000_000_000, 1_700_000_001_000]);
            expect((nodes[0] as { timestamp?: number }).timestamp).toBe(1_700_000_000_000);
            expect((nodes[1] as ToolNode).timestamp).toBe(1_700_000_001_000);
        });

        it("a tool_result replacement preserves the tool_call's stamp", () => {
            const lines = [
                line({ type: "tool_call", tool: "Bash", id: "tool-1", params: { command: "ls" } }),
                line({ type: "tool_result", tool: "Bash", id: "tool-1", status: "success", duration: 0.1 }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json", undefined, [111_000, 222_000]);
            const tool = nodes[0] as ToolNode;
            expect(tool.status).toBe("success");
            expect(tool.timestamp).toBe(111_000);
        });

        it("a 0 stamp entry means unknown and does not stamp the node", () => {
            const lines = [line({ type: "text", content: "hello" })];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json", undefined, [0]);
            const ts = (nodes[0] as { timestamp?: number }).timestamp;
            expect(ts === undefined || ts === 0).toBe(true);
            expect(ts).not.toBe(1970); // sanity: nothing invents a time
        });

        it("absent stamps array leaves behavior unchanged", () => {
            const lines = [line({ type: "text", content: "hello" })];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");
            expect(nodes).toHaveLength(1);
        });

        it("a wire timestamp wins over the batch stamp", () => {
            const wire = "2026-08-09T12:00:00Z";
            const lines = [
                JSON.stringify({
                    type: "system",
                    subtype: "agentmux_session_outcome",
                    outcome: "fresh",
                    attempted_sid: "sid-1",
                    actual_sid: null,
                    timestamp: wire,
                }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json", undefined, [999_000]);
            const outcome = nodes.find((n) => n.type === "session_outcome") as { timestamp: number };
            expect(outcome.timestamp).toBe(Date.parse(wire));
        });
    });

    // §3.5 of SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW
    // _2026_08_09.md: `fresh` outcomes materialize as divider nodes; `resumed`
    // outcomes are demoted — persisted line kept, no working-view node.
    describe("agentmux_session_outcome replay (session-scoped scrollback §3.5)", () => {
        const outcomeLine = (outcome: "fresh" | "resumed"): string =>
            JSON.stringify({
                type: "system",
                subtype: "agentmux_session_outcome",
                outcome,
                attempted_sid: "sid-1",
                actual_sid: null,
                timestamp: "2026-08-09T12:00:00Z",
            });

        it("materializes a fresh outcome as a session_outcome node", () => {
            const lines = [
                line({ type: "text", content: "before" }),
                outcomeLine("fresh"),
                line({ type: "text", content: "after" }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");
            const outcomes = nodes.filter((n) => n.type === "session_outcome");
            expect(outcomes).toHaveLength(1);
            expect((outcomes[0] as { outcome: string }).outcome).toBe("fresh");
        });

        it("does NOT materialize a resumed outcome (demoted, §3.5)", () => {
            const lines = [
                line({ type: "text", content: "before" }),
                outcomeLine("resumed"),
                line({ type: "text", content: "after" }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");
            expect(nodes.some((n) => n.type === "session_outcome")).toBe(false);
            // The surrounding conversation still replays.
            expect(nodes.filter((n) => n.type === "markdown").length).toBeGreaterThan(0);
        });

        it("materializes resumed outcomes when includeResumedOutcomes is set (Agent History view, §4.1)", () => {
            const lines = [outcomeLine("resumed")];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json", undefined, undefined, {
                includeResumedOutcomes: true,
            });
            const outcomes = nodes.filter((n) => n.type === "session_outcome");
            expect(outcomes).toHaveLength(1);
            expect((outcomes[0] as { outcome: string }).outcome).toBe("resumed");
        });
    });

    /**
     * reagentx P1, PR #3502, second review round: a hidden memory-
     * reinjection turn (memory-reinjection.ts's composeReinjectionMessage)
     * landing as the LAST turn of a session, immediately before an
     * `agentmux_session_outcome` boundary, must not suppress the NEXT
     * session's real content. Before the fix, `hidingUntilNextUserMessage`
     * stayed stuck `true` across the boundary (nothing reset it), silently
     * dropping every event of the following session from the replayed
     * transcript until some future real user_message happened to appear.
     */
    describe("hidden memory-reinjection state resets at a session boundary (reagentx P1, PR #3502)", () => {
        const outcomeLine = (outcome: "fresh" | "resumed"): string =>
            JSON.stringify({
                type: "system",
                subtype: "agentmux_session_outcome",
                outcome,
                attempted_sid: "sid-1",
                actual_sid: null,
                timestamp: "2026-08-09T12:00:00Z",
            });

        const hiddenReinjectionLine = (): string =>
            line({
                type: "user_message",
                message: composeReinjectionMessage(
                    [{ label: "g1", source: "global", body: "global body", sizeBytes: 11 }],
                    "compaction",
                ),
            });

        it("a session boundary after a hidden turn does not swallow the NEXT session's real content", () => {
            const lines = [
                hiddenReinjectionLine(),
                line({ type: "text", content: "the model's real reply to the hidden turn" }),
                outcomeLine("fresh"),
                // Next session's genuine content — must NOT be suppressed.
                line({ type: "text", content: "real content from the next session" }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");

            const markdownTexts = nodes.filter((n) => n.type === "markdown").map((n) => (n as { content: string }).content);
            expect(markdownTexts).toContain("real content from the next session");
        });

        it("also resets across a 'resumed' boundary, not just 'fresh'", () => {
            const lines = [
                hiddenReinjectionLine(),
                outcomeLine("resumed"),
                line({ type: "text", content: "real content after resume" }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");

            const markdownTexts = nodes.filter((n) => n.type === "markdown").map((n) => (n as { content: string }).content);
            expect(markdownTexts).toContain("real content after resume");
        });

        it("still suppresses correctly WITHIN one session — no regression to the base fix", () => {
            const lines = [
                hiddenReinjectionLine(),
                line({ type: "text", content: "the model's real reply — must stay hidden" }),
                line({ type: "user_message", message: "a real, ordinary next message" }),
                line({ type: "text", content: "visible again after the real user message" }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");

            const markdownTexts = nodes.filter((n) => n.type === "markdown").map((n) => (n as { content: string }).content);
            expect(markdownTexts).not.toContain("the model's real reply — must stay hidden");
            expect(markdownTexts).toContain("visible again after the real user message");
        });
    });

    describe("compact_boundary replay (Codex P2, PR #2378 round 2)", () => {
        // Before this fix, a raw `system`/`compact_boundary` frame had no
        // StreamEvent shape in the provider translator, so parseHistoryLines
        // silently dropped it — every historical compaction record vanished
        // from a reopened pane's transcript even though the live pane had
        // shown it correctly at the time.

        function compactBoundaryLine(metadataOverrides: Record<string, unknown> = {}): string {
            return JSON.stringify({
                type: "system",
                subtype: "compact_boundary",
                content: "Conversation compacted",
                level: "info",
                compactMetadata: {
                    trigger: "manual",
                    preTokens: 783_887,
                    postTokens: 11_775,
                    cumulativeDroppedTokens: 772_112,
                    durationMs: 231_606,
                    ...metadataOverrides,
                },
                timestamp: "2026-07-21T17:55:35.500Z",
            });
        }

        it("rebuilds a context_compacted node with the real trigger/token/duration data", () => {
            const lines = [
                line({ type: "text", content: "before" }),
                compactBoundaryLine(),
                line({ type: "text", content: "after" }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");
            const compacted = nodes.find((n) => n.type === "context_compacted") as any;
            expect(compacted).toBeDefined();
            expect(compacted).toMatchObject({
                type: "context_compacted",
                tokensBefore: 783_887,
                tokensAfter: 11_775,
                source: "real",
                trigger: "manual",
                durationMs: 231_606,
            });
            expect(compacted.timestamp).toBe(Date.parse("2026-07-21T17:55:35.500Z"));
        });

        it("preserves insertion order relative to surrounding text", () => {
            const lines = [
                line({ type: "text", content: "before" }),
                compactBoundaryLine(),
                line({ type: "text", content: "after" }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");
            expect(nodes.map((n) => n.type)).toEqual(["markdown", "context_compacted", "markdown"]);
        });

        it("rebuilds an auto-triggered boundary too", () => {
            const lines = [compactBoundaryLine({ trigger: "auto" })];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");
            expect((nodes[0] as any).trigger).toBe("auto");
        });

        it("drops a compact_boundary frame with malformed compactMetadata rather than emitting a bad node", () => {
            const lines = [
                line({ type: "text", content: "before" }),
                compactBoundaryLine({ preTokens: "not-a-number" }),
                line({ type: "text", content: "after" }),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");
            // The dropped frame emits no node at all (not even a bad one) — it's
            // simply absent from the replay, same as it never existed. Whether
            // the surrounding text merges into one markdown block or stays two
            // is the underlying parser's ordinary adjacent-text behavior, not
            // something this fix changes; the invariant under test is just that
            // no context_compacted node was fabricated from malformed data.
            expect(nodes.some((n) => n.type === "context_compacted")).toBe(false);
        });

        it("dedupes a replayed identical compact_boundary line", () => {
            const lines = [compactBoundaryLine(), compactBoundaryLine()];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");
            expect(nodes.filter((n) => n.type === "context_compacted")).toHaveLength(1);
        });

        it("keys a timestamp-less boundary's id the same way the live path would (codex P2, round 12)", () => {
            // Constructed without a top-level `timestamp` field -- the
            // defensive fallback case. Before round 12 this used a
            // batch-relative `nodes.length` counter here, while
            // useAgentStream.ts's live path used `Date.now()`; the same
            // underlying boundary seen live AND via a history-replay
            // overlap could then get two different ids and show up twice.
            const raw = JSON.parse(compactBoundaryLine());
            delete raw.timestamp;
            const lines = [
                line({ type: "text", content: "before" }),
                JSON.stringify(raw),
            ];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");
            const compacted = nodes.find((n) => n.type === "context_compacted") as any;
            expect(compacted).toBeDefined();
            expect(compacted.id).toBe(
                contextCompactedNodeId({
                    trigger: "manual",
                    preTokens: 783_887,
                    postTokens: 11_775,
                    durationMs: 231_606,
                    uuid: null,
                }),
            );
        });

        /** The stdout form, as Claude Code 2.1.287 writes it: snake_case, no `timestamp`. */
        function stdoutBoundary(trigger: "manual" | "auto", uuid = "8c1f4e2a-2b7d-4a51-9a0e-6f3c2d1b0a99") {
            return {
                type: "system",
                subtype: "compact_boundary",
                uuid,
                compact_metadata: {
                    trigger,
                    pre_tokens: 25040,
                    post_tokens: 733,
                    cumulative_dropped_tokens: 24307,
                    duration_ms: 1513,
                },
                logical_parent_uuid: "3e9b7c10-5d2f-4c8a-b1e4-7a6d5c4b3a21",
            };
        }

        it.each(["manual", "auto"] as const)("rebuilds the real stdout (snake_case) boundary — %s", (trigger) => {
            const lines = [line({ type: "text", content: "before" }), JSON.stringify(stdoutBoundary(trigger))];
            const { nodes } = parseHistoryLines(lines, "claude-stream-json");
            const compacted = nodes.find((n) => n.type === "context_compacted") as any;
            expect(compacted).toMatchObject({
                id: "context-compacted-8c1f4e2a-2b7d-4a51-9a0e-6f3c2d1b0a99",
                tokensBefore: 25040,
                tokensAfter: 733,
                source: "real",
                trigger,
                durationMs: 1513,
            });
        });

        it("gives a boundary the same id live and on replay, and two boundaries different ids", () => {
            const replayIds = (frame: object) =>
                parseHistoryLines([JSON.stringify(frame)], "claude-stream-json").nodes.map((n) => n.id);
            // The live path: the reducer's event, keyed as useAgentStream's pushContextCompactedNodes keys it.
            const liveId = (frame: ReturnType<typeof stdoutBoundary>) => {
                const { events } = update(initialState("agent"), {
                    type: "CompactionBoundary",
                    trigger: frame.compact_metadata.trigger,
                    preTokens: frame.compact_metadata.pre_tokens,
                    postTokens: frame.compact_metadata.post_tokens,
                    durationMs: frame.compact_metadata.duration_ms,
                    at: 1,
                    frameTimestamp: null,
                    boundaryUuid: frame.uuid,
                });
                const ev = events.find((e) => e.type === "context-compacted") as any;
                return contextCompactedNodeId({ ...ev, preTokens: ev.tokensBefore, postTokens: ev.tokensAfter, uuid: ev.boundaryUuid });
            };
            const first = stdoutBoundary("auto");
            const second = stdoutBoundary("auto", "0f2e4d6c-8b0a-4c1e-9f3d-5a7b9c1d3e5f");
            expect(replayIds(first)).toEqual([liveId(first)]);
            expect(replayIds(second)).toEqual([liveId(second)]);
            expect(liveId(first)).not.toBe(liveId(second));
        });

        it("never triggers a memory reinjection on replay", () => {
            parseHistoryLines([JSON.stringify(stdoutBoundary("manual"))], "claude-stream-json");
            expect(vi.mocked(createMemoryReinjectionController)).not.toHaveBeenCalled();
        });
    });
});

describe("parseHistoryLines — Gemini-family transcripts keep the user's messages", () => {
    it("restores the user message from the CLI's echo", () => {
        const lines = [
            JSON.stringify({ type: "init", session_id: "s", model: "gemini" }),
            JSON.stringify({ type: "message", role: "user", content: "summarise the repo" }),
            JSON.stringify({ type: "message", role: "assistant", content: "Here is", delta: true }),
            JSON.stringify({ type: "message", role: "assistant", content: " a summary.", delta: true }),
            JSON.stringify({ type: "result", status: "success", stats: {} }),
        ];
        const { nodes } = parseHistoryLines(lines, "gemini-json");
        const kinds = nodes.map((n) => n.type);
        expect(kinds[0]).toBe("user_message");
        expect((nodes[0] as { message: string }).message).toBe("summarise the repo");
        expect(kinds).toContain("markdown");
    });
});

describe("parseHistoryLines — restored user messages keep their historical time (ReAgent P1, #3620)", () => {
    // A translator must not invent a "now" timestamp for a replayed user
    // message: parseHistoryLines only fills the line's batch stamp when a node
    // has no timestamp, so a fabricated Date.now() showed every restored user
    // message as sent "just now".
    const T0 = Date.UTC(2026, 0, 2, 3, 4, 5);

    it("Gemini: the echo takes its line's batch stamp", () => {
        const lines = [
            JSON.stringify({ type: "message", role: "user", content: "hi" }),
            JSON.stringify({ type: "message", role: "assistant", content: "hello", delta: true }),
        ];
        const { nodes } = parseHistoryLines(lines, "gemini-json", undefined, [T0, T0 + 1000]);
        const user = nodes.find((n) => n.type === "user_message") as { timestamp?: number };
        expect(user.timestamp).toBe(T0);
    });

    it("Claude: a persisted user line takes its line's batch stamp", () => {
        const lines = [
            JSON.stringify({ type: "user", message: { role: "user", content: "hi" } }),
            JSON.stringify({ type: "assistant", message: { role: "assistant", content: [{ type: "text", text: "hello" }] } }),
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json", undefined, [T0, T0 + 1000]);
        const user = nodes.find((n) => n.type === "user_message") as { timestamp?: number };
        expect(user.timestamp).toBe(T0);
    });

    it("no stamp available: the timestamp is unknown, never a fabricated 'now'", () => {
        const before = Date.now();
        const { nodes } = parseHistoryLines([JSON.stringify({ type: "message", role: "user", content: "hi" })], "gemini-json");
        const user = nodes.find((n) => n.type === "user_message") as { timestamp?: number };
        expect(user.timestamp === undefined || user.timestamp < before).toBe(true);
    });
});

describe("HistoryParser (resumable parseHistoryLines)", () => {
    // A transcript exercising every piece of state that spans lines: text and
    // thinking runs, a tool whose result lands later, a compaction boundary
    // and a session outcome (both bypass the translator and flush the parser),
    // usage-bearing session_end, and several turns.
    const transcript = [
        line({ type: "user_message", message: "first question" }),
        line({ type: "thinking", content: "Let me " }),
        line({ type: "thinking", content: "Let me think" }),
        line({ type: "text", content: "Looking " }),
        line({ type: "text", content: "Looking at it" }),
        line({ type: "tool_call", tool: "Read", id: "tool-a", params: { file_path: "a.ts" } }),
        line({ type: "text", content: "while it reads" }),
        line({ type: "tool_result", tool: "Read", id: "tool-a", status: "success", duration: 0.2 }),
        messageStart("msg_a", { input_tokens: 3, cache_read_input_tokens: 1_000 }),
        line({ type: "session_end", stats: { input_tokens: 120, output_tokens: 12 } }),
        JSON.stringify({
            type: "system",
            subtype: "compact_boundary",
            content: "Conversation compacted",
            level: "info",
            compactMetadata: { trigger: "auto", preTokens: 1000, postTokens: 100, durationMs: 50 },
            timestamp: "2026-09-24T10:00:00Z",
        }),
        line({ type: "user_message", message: "second question" }),
        line({ type: "text", content: "Answer " }),
        line({ type: "text", content: "Answer two" }),
        JSON.stringify({
            type: "system",
            subtype: "agentmux_session_outcome",
            outcome: "resumed",
            attempted_sid: "sid-1",
            actual_sid: "sid-1",
            timestamp: "2026-09-24T10:05:00Z",
        }),
        line({ type: "user_message", message: "third question" }),
        line({ type: "tool_call", tool: "Bash", id: "tool-b", params: { command: "ls" } }),
        line({ type: "tool_result", tool: "Bash", id: "tool-b", status: "success", duration: 0.1 }),
        line({ type: "text", content: "done" }),
        messageStart("msg_b", { input_tokens: 3, cache_read_input_tokens: 2_000 }),
        resultFrame(4_000),
        line({ type: "session_end", stats: { input_tokens: 300, output_tokens: 30 } }),
    ];
    const stamps = transcript.map((_, i) => 1_790_000_000_000 + i * 1000);
    const opts = { includeResumedOutcomes: true };

    const feedInChunks = (cuts: number[]) => {
        const parser = new HistoryParser("claude-stream-json", "me", opts);
        let from = 0;
        for (const to of [...cuts, transcript.length]) {
            parser.feed(transcript.slice(from, to), stamps.slice(from, to));
            from = to;
        }
        return parser;
    };

    it("feeding in any two chunks gives exactly the nodes of one parse", () => {
        const whole = parseHistoryLines(transcript, "claude-stream-json", "me", stamps, opts);
        expect(whole.nodes.length).toBeGreaterThan(8);
        for (let cut = 0; cut <= transcript.length; cut++) {
            const parser = feedInChunks([cut]);
            expect(parser.nodes, `cut at ${cut}`).toEqual(whole.nodes);
            expect(parser.lastContext, `cut at ${cut}`).toEqual(whole.lastContext);
            expect(parser.reportedContextWindows, `cut at ${cut}`).toEqual(whole.reportedContextWindows);
        }
    });

    it("feeding line by line gives exactly the nodes of one parse", () => {
        const whole = parseHistoryLines(transcript, "claude-stream-json", "me", stamps, opts);
        const parser = feedInChunks(transcript.map((_, i) => i).slice(1));
        expect(parser.nodes).toEqual(whole.nodes);
    });

    it("reports the ids each feed added or replaced", () => {
        const parser = new HistoryParser("claude-stream-json", "me", opts);
        parser.feed(transcript.slice(0, 6), stamps.slice(0, 6));
        const toolIndex = parser.nodes.findIndex((n) => n.id === "tool-a");
        expect(toolIndex).toBeGreaterThanOrEqual(0);
        // The tool's result arrives in the next chunk: the tool is reported
        // as changed and stays where its call appeared.
        const changed = parser.feed(transcript.slice(6, 8), stamps.slice(6, 8));
        expect(changed.has("tool-a")).toBe(true);
        expect(parser.nodes[toolIndex].id).toBe("tool-a");
        expect((parser.nodes[toolIndex] as ToolNode).status).toBe("success");
    });
});

describe("user messages persisted for CLIs that don't echo them (spec §6.9)", () => {
    // What the per-turn subprocess controller writes before each turn
    // (agentmux-srv subprocess::user_record) — the same line the persistent
    // Claude controller writes for stdin.
    const userRecord = (content: string) => JSON.stringify({ type: "user", message: { role: "user", content } });

    it.each(["codex-json", "kimi-stream-json", "agy-stream-json", "claude-stream-json"])(
        "%s: replays the record as the user's message",
        (format) => {
            const { nodes } = parseHistoryLines([userRecord("fix the build")], format);
            const user = nodes.find((n) => n.type === "user_message") as { message?: string } | undefined;
            expect(user?.message).toBe("fix the build");
        }
    );

    it.each(["codex-json", "kimi-stream-json", "agy-stream-json"])(
        "%s: a live translator doesn't render it (the pane already shows it)",
        async (format) => {
            const { createTranslator } = await import("./providers/translator-factory");
            expect(createTranslator(format).translate(JSON.parse(userRecord("hi")))).toEqual([]);
        }
    );
});

// SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md §2.2 — replay places a jekt that
// arrived mid-block the same way the live pane does.
describe("parseHistoryLines — jekt arriving mid-block", () => {
    const jekt =
        "[JEKT:FROM=reagent TO=lark TIER=coord DELIVERY=wan TRUST=network-claimed MSGID=m1 PRIORITY=normal TS=1783386012]\n" +
        "From: reagent | To: lark | ts=1783386012\nPR reviewed\n[/JEKT]";

    it("keeps the text block whole and puts the jekt after it, before the tool that ended it", () => {
        const lines = [
            line({ type: "text", content: "first half " }),
            line({ type: "user_message", message: jekt }),
            line({ type: "text", content: "second half" }),
            line({ type: "tool_call", tool: "Bash", id: "tool-1", params: { command: "ls" } }),
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes.map((n) => n.type)).toEqual(["markdown", "jekt_message", "tool"]);
        expect((nodes[0] as { content: string }).content).toBe("first half second half");
    });

    it("a batch that ends mid-block still shows the held jekt", () => {
        const lines = [
            line({ type: "text", content: "still writing" }),
            line({ type: "user_message", message: jekt }),
        ];
        const { nodes } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes.map((n) => n.type)).toEqual(["markdown", "jekt_message"]);
    });
});

describe("parseHistoryLines — Claude Code's compaction summary (SPEC_CONTEXT_DELIVERY_2026_09_30 §3.3)", () => {
    const SUMMARY = [
        "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.",
        "",
        "Summary:",
        "1. Primary Request and Intent:",
        "   Fix the console sign-in.",
    ].join("\n");
    const boundaryLine = JSON.stringify({
        type: "system",
        subtype: "compact_boundary",
        compactMetadata: { trigger: "manual", preTokens: 120_000, postTokens: 8_000, durationMs: 30_000 },
        timestamp: "2026-09-30T08:00:00.000Z",
    });
    const summaryLine = JSON.stringify({
        type: "user",
        message: { role: "user", content: SUMMARY },
        isSynthetic: true,
        uuid: "summary-uuid-1",
        timestamp: "2026-09-30T08:00:01.000Z",
    });

    it("renders the summary as a context delivery, not a user message", () => {
        const { nodes } = parseHistoryLines(
            [line({ type: "text", content: "before" }), boundaryLine, summaryLine],
            "claude-stream-json",
        );
        expect(nodes.map((n) => n.type)).toEqual(["markdown", "context_compacted", "context_delivery"]);
        const card = nodes[2] as any;
        expect(card.reason).toBe("compaction");
        expect(card.trigger).toBe("manual");
        expect(card.items[0]).toMatchObject({ kind: "compaction_summary", body: SUMMARY, excerpt: "Fix the console sign-in." });
    });

    it("recognises a summary at the top of a page whose boundary is on the older page", () => {
        // useHistoryPagination parses each page separately, newest first.
        const newer = parseHistoryLines([summaryLine], "claude-stream-json").nodes;
        const older = parseHistoryLines([boundaryLine], "claude-stream-json").nodes;
        expect(newer.map((n) => n.type)).toEqual(["context_delivery"]);
        expect(older.map((n) => n.type)).toEqual(["context_compacted"]);
        // Same id as when both are on one page, so the document store dedupes.
        const together = parseHistoryLines([boundaryLine, summaryLine], "claude-stream-json").nodes;
        expect(newer[0].id).toBe(together[1].id);
    });

    it("leaves the same text a user message when it's neither flagged nor after a boundary", () => {
        const unflagged = JSON.stringify({ type: "user", message: { role: "user", content: SUMMARY } });
        const { nodes } = parseHistoryLines([unflagged], "claude-stream-json");
        expect(nodes.map((n) => n.type)).toEqual(["user_message"]);
    });

    it("keeps one card when the same lines replay twice", () => {
        const { nodes } = parseHistoryLines([boundaryLine, summaryLine, boundaryLine, summaryLine], "claude-stream-json");
        expect(nodes.filter((n) => n.type === "context_delivery")).toHaveLength(1);
        expect(nodes.filter((n) => n.type === "user_message")).toHaveLength(0);
    });

    it("pairs a boundary and summary that land in different history pages", () => {
        const parser = new HistoryParser("claude-stream-json", "me");
        parser.feed([boundaryLine]);
        parser.feed([summaryLine]);
        expect(parser.nodes.map((n) => n.type)).toEqual(["context_compacted", "context_delivery"]);
    });
});

describe("parseHistoryLines — tool result source line (SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01 §3.2)", () => {
    const toolUse = JSON.stringify({ type: "assistant", message: { content: [{ type: "tool_use", id: "tu1", name: "Bash", input: { command: "ls" } }] } });
    const toolResult = JSON.stringify({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "tu1", content: "a\nb" }] } });

    it("records the transcript line a tool's result came from", () => {
        const { nodes } = parseHistoryLines(["", toolUse, toolResult], "claude-stream-json", undefined, undefined, {
            source: { stream: "b:blk", gen: "g1", firstLine: 100 },
        });
        const tool = nodes.find((n) => n.type === "tool") as any;
        expect(tool.resultSource).toEqual({ stream: "b:blk", gen: "g1", line: 102 });
    });

    it("leaves it unset when the caller doesn't know the source", () => {
        const { nodes } = parseHistoryLines([toolUse, toolResult], "claude-stream-json");
        expect((nodes.find((n) => n.type === "tool") as any).resultSource).toBeUndefined();
    });
});
