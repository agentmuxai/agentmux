// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach } from "vitest";
import { ClaudeCodeStreamParser, STARTUP_HEADING_RE } from "./stream-parser";
import { buildStartupPayload } from "./startup/buildStartupPayload";
import type { StreamEvent, MarkdownNode, ToolNode, UserMessageNode } from "./types";

let parser: ClaudeCodeStreamParser;

beforeEach(() => {
    parser = new ClaudeCodeStreamParser();
});

// ── Text accumulation ───────────────────────────────────────────────────────

describe("text accumulation", () => {
    test("consecutive text events produce same node ID with accumulated content", () => {
        const n1 = parser.parseStreamEvent({ type: "text", content: "Hello " });
        const n2 = parser.parseStreamEvent({ type: "text", content: "world" });
        const n3 = parser.parseStreamEvent({ type: "text", content: "!" });

        expect(n1).not.toBeNull();
        expect(n2).not.toBeNull();
        expect(n3).not.toBeNull();

        // All three share the same ID
        expect(n1!.id).toBe(n2!.id);
        expect(n2!.id).toBe(n3!.id);

        // Content accumulates
        expect((n1 as MarkdownNode).content).toBe("Hello ");
        expect((n2 as MarkdownNode).content).toBe("Hello world");
        expect((n3 as MarkdownNode).content).toBe("Hello world!");
    });

    test("text after a tool_call gets a new node ID", () => {
        const t1 = parser.parseStreamEvent({ type: "text", content: "Before" });
        parser.parseStreamEvent({
            type: "tool_call",
            tool: "Bash",
            id: "tc_1",
            params: { command: "ls" },
        });
        const t2 = parser.parseStreamEvent({ type: "text", content: "After" });

        expect(t1!.id).not.toBe(t2!.id);
        expect((t2 as MarkdownNode).content).toBe("After");
    });

    test("text after a tool_result gets a new node ID", () => {
        const t1 = parser.parseStreamEvent({ type: "text", content: "Before" });
        parser.parseStreamEvent({
            type: "tool_call",
            tool: "Read",
            id: "tc_2",
            params: { file_path: "test.ts" },
        });
        parser.parseStreamEvent({
            type: "tool_result",
            tool: "Read",
            id: "tc_2",
            status: "success",
            duration: 0.1,
        });
        const t2 = parser.parseStreamEvent({ type: "text", content: "After" });

        expect(t1!.id).not.toBe(t2!.id);
    });

    test("text after user_message gets a new node ID", () => {
        const t1 = parser.parseStreamEvent({ type: "text", content: "Response 1" });
        parser.parseStreamEvent({ type: "user_message", message: "Hello" });
        const t2 = parser.parseStreamEvent({ type: "text", content: "Response 2" });

        expect(t1!.id).not.toBe(t2!.id);
    });
});

// ── Thinking accumulation ───────────────────────────────────────────────────

describe("thinking accumulation", () => {
    test("consecutive thinking events produce same node ID with accumulated content", () => {
        const n1 = parser.parseStreamEvent({ type: "thinking", content: "Let me " });
        const n2 = parser.parseStreamEvent({ type: "thinking", content: "think..." });

        expect(n1!.id).toBe(n2!.id);
        expect((n1 as MarkdownNode).content).toBe("Let me ");
        expect((n2 as MarkdownNode).content).toBe("Let me think...");
        expect((n2 as MarkdownNode).metadata?.thinking).toBe(true);
    });

    test("thinking after text gets a new node ID", () => {
        const text = parser.parseStreamEvent({ type: "text", content: "Hello" });
        const think = parser.parseStreamEvent({ type: "thinking", content: "Hmm" });

        expect(text!.id).not.toBe(think!.id);
        expect((think as MarkdownNode).metadata?.thinking).toBe(true);
    });

    test("text after thinking gets a new node ID", () => {
        const think = parser.parseStreamEvent({ type: "thinking", content: "Hmm" });
        const text = parser.parseStreamEvent({ type: "text", content: "Result" });

        expect(think!.id).not.toBe(text!.id);
        expect((text as MarkdownNode).metadata).toBeUndefined();
    });
});

// ── Interleaved events ──────────────────────────────────────────────────────

describe("interleaved events", () => {
    test("text → tool_call → text → thinking → text produces distinct nodes", () => {
        const ids = new Set<string>();

        const t1 = parser.parseStreamEvent({ type: "text", content: "A" });
        ids.add(t1!.id);

        // Second text delta appends to t1
        const t1b = parser.parseStreamEvent({ type: "text", content: "B" });
        expect(t1b!.id).toBe(t1!.id); // Same node

        const tool = parser.parseStreamEvent({
            type: "tool_call",
            tool: "Bash",
            id: "tc_x",
            params: { command: "echo hi" },
        });
        ids.add(tool!.id);

        const t2 = parser.parseStreamEvent({ type: "text", content: "C" });
        ids.add(t2!.id);

        const think = parser.parseStreamEvent({ type: "thinking", content: "D" });
        ids.add(think!.id);

        const t3 = parser.parseStreamEvent({ type: "text", content: "E" });
        ids.add(t3!.id);

        // 5 distinct nodes: t1(AB), tool, t2(C), think(D), t3(E)
        expect(ids.size).toBe(5);
    });

    test("agent_message breaks text accumulation", () => {
        const t1 = parser.parseStreamEvent({ type: "text", content: "Before" });
        parser.parseStreamEvent({
            type: "agent_message",
            from: "agent1",
            to: "agent2",
            message: "hello",
            method: "mux",
        });
        const t2 = parser.parseStreamEvent({ type: "text", content: "After" });

        expect(t1!.id).not.toBe(t2!.id);
    });
});

// ── parseStreamEvent ────────────────────────────────────────────────────────

describe("parseStreamEvent", () => {
    test("returns null for unknown event type", () => {
        const node = parser.parseStreamEvent({ type: "unknown" } as any);
        expect(node).toBeNull();
    });

    test("returns tool node for tool_call", () => {
        const node = parser.parseStreamEvent({
            type: "tool_call",
            tool: "Read",
            id: "tc_r",
            params: { file_path: "foo.ts" },
        });
        expect(node).not.toBeNull();
        expect(node!.type).toBe("tool");
        expect((node as ToolNode).status).toBe("running");
    });

    test("tool_result updates pending tool call", () => {
        parser.parseStreamEvent({
            type: "tool_call",
            tool: "Bash",
            id: "tc_b",
            params: { command: "ls" },
        });
        const result = parser.parseStreamEvent({
            type: "tool_result",
            tool: "Bash",
            id: "tc_b",
            status: "success",
            duration: 0.5,
        });

        expect(result).not.toBeNull();
        expect((result as ToolNode).status).toBe("success");
        expect((result as ToolNode).id).toBe("tc_b");
        expect((result as ToolNode).duration).toBe(0.5);
    });
});

// ── parseLine ───────────────────────────────────────────────────────────────

describe("parseLine", () => {
    test("parses valid JSON line", () => {
        const node = parser.parseLine('{"type":"text","content":"hello"}');
        expect(node).not.toBeNull();
        expect((node as MarkdownNode).content).toBe("hello");
    });

    test("returns null for empty line", () => {
        expect(parser.parseLine("")).toBeNull();
        expect(parser.parseLine("   ")).toBeNull();
    });

    test("returns null for invalid JSON", () => {
        expect(parser.parseLine("not json")).toBeNull();
    });
});

// ── flushPending ────────────────────────────────────────────────────────────

describe("flushPending", () => {
    test("returns empty array when nothing accumulated", () => {
        expect(parser.flushPending()).toEqual([]);
    });

    test("returns accumulated text node", () => {
        parser.parseStreamEvent({ type: "text", content: "hello" });
        const flushed = parser.flushPending();
        expect(flushed).toHaveLength(1);
        expect((flushed[0] as MarkdownNode).content).toBe("hello");
    });

    test("returns accumulated thinking node", () => {
        parser.parseStreamEvent({ type: "thinking", content: "hmm" });
        const flushed = parser.flushPending();
        expect(flushed).toHaveLength(1);
        expect((flushed[0] as MarkdownNode).metadata?.thinking).toBe(true);
    });

    test("clears accumulators after flush", () => {
        parser.parseStreamEvent({ type: "text", content: "hello" });
        parser.flushPending();
        expect(parser.flushPending()).toEqual([]);
    });
});

// ── tool_chunk ─────────────────────────────────────────────────────────────

describe("tool_chunk parsing", () => {
    test("eventToNode returns null for tool_chunk (no DocumentNode)", () => {
        const node = parser.parseStreamEvent({
            type: "tool_chunk",
            id: "tc_x",
            kind: "stdout",
            content: "hi",
            timestamp: 42,
        } as StreamEvent);
        expect(node).toBeNull();
    });

    test("parseToolChunkEvent normalizes the event into a chunk record", () => {
        const out = parser.parseToolChunkEvent({
            type: "tool_chunk",
            id: "tc_x",
            kind: "stderr",
            content: "warn\n",
            timestamp: 99,
        });
        expect(out.toolId).toBe("tc_x");
        expect(out.chunk).toEqual({
            kind: "stderr",
            content: "warn\n",
            timestamp: 99,
        });
    });

    test("parseToolChunkEvent defaults timestamp to the injected now", () => {
        const out = parser.parseToolChunkEvent(
            {
                type: "tool_chunk",
                id: "tc_x",
                kind: "stdout",
                content: "hi",
            },
            12345,
        );
        expect(out.chunk.timestamp).toBe(12345);
    });

    test("tool_chunk does not disturb text accumulation", () => {
        const t1 = parser.parseStreamEvent({ type: "text", content: "Hello " });
        const chunkNode = parser.parseStreamEvent({
            type: "tool_chunk",
            id: "tc_y",
            kind: "stdout",
            content: "ignore me",
        } as StreamEvent);
        const t2 = parser.parseStreamEvent({ type: "text", content: "world" });
        expect(chunkNode).toBeNull();
        // Text node IDs stay equal — tool_chunk did NOT break accumulation.
        expect(t1!.id).toBe(t2!.id);
        expect((t2 as MarkdownNode).content).toBe("Hello world");
    });
});

// ── reset ───────────────────────────────────────────────────────────────────

describe("reset", () => {
    test("clears all accumulation state", () => {
        parser.parseStreamEvent({ type: "text", content: "hello" });
        parser.reset();

        // After reset, a new text event starts a fresh node — id format
        // is `node_${prefix}_${counter}` where the prefix is rotated on
        // reset, so we can't pin the exact value. Just assert the shape
        // and that content is the fresh string.
        const node = parser.parseStreamEvent({ type: "text", content: "fresh" });
        expect(node!.id).toMatch(/^node_[a-z0-9]+_0$/);
        expect((node as MarkdownNode).content).toBe("fresh");
    });
});

// ── startup-injection detection (SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24) ─

describe("user_message startup-injection detection", () => {
    test("sets isStartup=true when message starts with '# Session Context'", () => {
        const node = parser.parseStreamEvent({
            type: "user_message",
            message: "# Session Context\n\n## Identity\n- Name: AgentA\n",
        });
        expect(node).not.toBeNull();
        expect(node!.type).toBe("user_message");
        expect((node as UserMessageNode).isStartup).toBe(true);
    });

    test("sets isStartup=false for normal typed input", () => {
        const node = parser.parseStreamEvent({
            type: "user_message",
            message: "Can you run the tests?",
        });
        expect((node as UserMessageNode).isStartup).toBe(false);
    });

    test("sets isStartup=false when '# Session Context' is not at the start", () => {
        // A user replying to an agent's output that quoted the heading
        // must NOT be misclassified as a startup payload.
        const node = parser.parseStreamEvent({
            type: "user_message",
            message: "I noticed the agent said:\n# Session Context\n— what does that mean?",
        });
        expect((node as UserMessageNode).isStartup).toBe(false);
    });

    test("sets isStartup=false when heading is similar but different", () => {
        // Word-boundary anchor `\b` after the literal — `# Session Contextual`
        // is NOT a startup payload.
        const node = parser.parseStreamEvent({
            type: "user_message",
            message: "# Session Contextual notes follow",
        });
        expect((node as UserMessageNode).isStartup).toBe(false);
    });

    test("STARTUP_HEADING_RE pins the literal heading buildStartupPayload emits", () => {
        // Contract test: any future rename of the heading in
        // buildStartupPayload.ts must update the regex in
        // stream-parser.ts in the same commit, or this test fails.
        const payload = buildStartupPayload({
            agent: {
                id: "test-agent",
                slug: "test",
                name: "Test",
                provider: "claude",
                description: "",
                icon: "",
                working_directory: "",
                provider_flags: "",
                shell: "",
                created_at: 0,
                updated_at: 0,
                agent_type: "host",
                environment: "",
                agent_bus_id: "",
                accounts: "",
                is_seeded: 0,
                parent_id: "",
                branch_label: "",
                user_hidden: 0,
            } as any,
            providerDisplayName: "Claude",
            workDir: "/tmp",
            version: "0.0.0",
            accounts: [],
            peerAgents: [],
            startupContent: null,
        });
        expect(payload).not.toBeNull();
        expect(STARTUP_HEADING_RE.test(payload!)).toBe(true);
    });
});
