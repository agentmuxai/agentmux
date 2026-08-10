// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { parseHistoryLines } from "./parseHistoryLines";
import { contextCompactedNodeId } from "./compact-boundary";
import type { ToolNode } from "./types";

// The Claude translator passes through events that already match the
// StreamEvent shape (`if (this.isStreamEvent(rawEvent))` branch), so
// each test line is JSON-stringified StreamEvent.
const line = (event: object): string => JSON.stringify(event);

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

    it("surfaces the last session_end stats without emitting a node for it", () => {
        const lines = [
            line({ type: "text", content: "hi" }),
            line({ type: "session_end", stats: { input_tokens: 111, output_tokens: 22 } }),
        ];
        const { nodes, lastSessionStats } = parseHistoryLines(lines, "claude-stream-json");
        expect(nodes).toHaveLength(1);
        expect(lastSessionStats).toEqual({ input_tokens: 111, output_tokens: 22 });
    });

    it("returns null lastSessionStats when no session_end is present", () => {
        const lines = [line({ type: "text", content: "hi" })];
        const { lastSessionStats } = parseHistoryLines(lines, "claude-stream-json");
        expect(lastSessionStats).toBeNull();
    });

    it("does not let a later empty-stats session_end clobber real historical stats", () => {
        // reagent P1 on PR #2059: Claude's persistent-mode controller emits a
        // session_end with stats: {} after EVERY plain-text turn (the per-turn
        // boundary marker) — the real usage-bearing `result` event only fires
        // at process teardown, which can be much earlier in the window. The
        // chronologically-last session_end here is the empty turn-boundary
        // marker; the real stats from the earlier turn must still win.
        const lines = [
            line({ type: "text", content: "turn one" }),
            line({ type: "session_end", stats: { input_tokens: 500, output_tokens: 50 } }),
            line({ type: "text", content: "turn two" }),
            line({ type: "session_end", stats: {} }),
        ];
        const { lastSessionStats } = parseHistoryLines(lines, "claude-stream-json");
        expect(lastSessionStats).toEqual({ input_tokens: 500, output_tokens: 50 });
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

        it("dedupes a replayed identical compact_boundary line by its frame timestamp", () => {
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
                    frameTimestamp: null,
                }),
            );
        });
    });
});
