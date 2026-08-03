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

    // SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md §2.4: thinking clumps
    // never got a timestamp at all before — a real gap found auditing for
    // the hover-peek feature, since every other node kind already had one.
    test("stamps a timestamp on first creation, unchanged across subsequent appends", () => {
        const before = Date.now();
        const n1 = parser.parseStreamEvent({ type: "thinking", content: "Let me " }) as MarkdownNode;
        const n2 = parser.parseStreamEvent({ type: "thinking", content: "think..." }) as MarkdownNode;

        expect(n1.timestamp).toBeGreaterThanOrEqual(before);
        expect(n2.timestamp).toBe(n1.timestamp);
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

        // After reset, a new text event should get a fresh node_0 ID.
        // Deterministic counter is the dedup contract — see
        // `parseHistoryLines` and the codex P1 on PR #1101.
        const node = parser.parseStreamEvent({ type: "text", content: "fresh" });
        expect(node!.id).toBe("node_0");
        expect((node as MarkdownNode).content).toBe("fresh");
    });
});

// ── skipIds (snapshot-collision avoidance) ─────────────────────────────────
// Render-gap fix on PR #1101: a fresh parser mounting against a resumed-
// session snapshot would generate `node_0` for its first text chunk, which
// the document reducer treats as "merge into existing node" — the response
// gets silently merged into the snapshot's old `node_0` 100+ positions
// back. The parser now accepts a `skipIds` set (the snapshot's id set) and
// advances its counter past anything already present.

describe("skipIds — snapshot-collision avoidance", () => {
    test("skips ids present in the snapshot set on first event", () => {
        const skip = new Set<string>(["node_0", "node_1", "node_2"]);
        const p = new ClaudeCodeStreamParser({ skipIds: skip });
        const node = p.parseStreamEvent({ type: "text", content: "first new" });
        expect(node!.id).toBe("node_3");
    });

    test("skip-set only affects matching id prefixes (msg_/user_ unaffected)", () => {
        // Snapshot has node_0..node_2; a new user_message should still
        // start its own `user_*` sequence at 0 because the counter is
        // shared but the prefix is different and only matching ids
        // are skipped. (Counter actually advances past node_*, so
        // user_3 is what we'd see if the agent already emitted three
        // `node_*` events first.)
        const p = new ClaudeCodeStreamParser({ skipIds: new Set(["user_0", "user_1"]) });
        const um = p.parseStreamEvent({
            type: "user_message",
            message: "hi",
        });
        expect(um!.id).toBe("user_2");
    });

    test("no skipIds → original counter-from-0 behavior preserved", () => {
        const p = new ClaudeCodeStreamParser();
        const a = p.parseStreamEvent({ type: "text", content: "a" });
        p.reset();
        const b = p.parseStreamEvent({ type: "text", content: "b" });
        expect(a!.id).toBe("node_0");
        expect(b!.id).toBe("node_0");
    });

    test("skipIds callback is read ON-DEMAND, not at construction (async snapshot race)", () => {
        // Codex P1 #2 on PR #1101: in resumed sessions the snapshot
        // restore is async — `HistoryLoaded` lands AFTER the parser
        // is constructed. A static skip-set captured at construction
        // would be empty, leaving the parser to collide with the
        // restored snapshot's ids. The callback form must read the
        // live set at each id generation.
        const liveSet = new Set<string>(); // initially empty
        const p = new ClaudeCodeStreamParser({ skipIds: () => liveSet });

        // Simulate: snapshot restore lands AFTER the parser was
        // constructed but BEFORE the first id is generated.
        liveSet.add("node_0");
        liveSet.add("node_1");

        const node = p.parseStreamEvent({ type: "text", content: "live" });
        expect(node!.id).toBe("node_2");
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

// ── [JEKT:...] marker detection (SPEC_JEKT_SECURITY_AND_VISIBILITY) ────────

describe("jekt marker detection", () => {
    const jektBlock = (overrides: Partial<{
        from: string; to: string; tier: string; delivery: string; trust: string;
        msgid: string; priority: string;
    }> = {}) => {
        const f = {
            from: "agentx", to: "agent3", tier: "coord", delivery: "host",
            trust: "host-verified", msgid: "abc123", priority: "normal",
            ...overrides,
        };
        return `[JEKT:FROM=${f.from} TO=${f.to} TIER=${f.tier} DELIVERY=${f.delivery} TRUST=${f.trust} MSGID=${f.msgid} PRIORITY=${f.priority} TS=1783386012]\n` +
            `────────────────────────────────────────────────────────────\n` +
            `From: ${f.from} | To: ${f.to} | ts=1783386012\n` +
            `Hey, can you review PR #25?\n` +
            `────────────────────────────────────────────────────────────\n` +
            `Reply: bus:inject to ${f.from}\n` +
            `[/JEKT]`;
    };

    test("parses a well-formed block into a jekt_message node", () => {
        const message = jektBlock();
        const node = parser.parseStreamEvent({ type: "user_message", message });
        expect(node).not.toBeNull();
        expect(node!.type).toBe("jekt_message");
        const jekt = node as import("./types").JektMessageNode;
        expect(jekt.from).toBe("agentx");
        expect(jekt.to).toBe("agent3");
        expect(jekt.tier).toBe("coord");
        expect(jekt.deliveryTier).toBe("host");
        expect(jekt.trust).toBe("host-verified");
        expect(jekt.msgId).toBe("abc123");
        expect(jekt.priority).toBe("normal");
        expect(jekt.raw).toBe(message);
    });

    test("strips divider/header/reply scaffolding from the displayed message", () => {
        const node = parser.parseStreamEvent({ type: "user_message", message: jektBlock() });
        const jekt = node as import("./types").JektMessageNode;
        expect(jekt.message).toBe("Hey, can you review PR #25?");
    });

    test("direction is incoming when TO matches the current agent", () => {
        parser.setAgentId("agent3");
        const node = parser.parseStreamEvent({ type: "user_message", message: jektBlock({ to: "agent3" }) });
        expect((node as import("./types").JektMessageNode).direction).toBe("incoming");
    });

    test("direction is outgoing when FROM matches the current agent and TO doesn't", () => {
        parser.setAgentId("agentx");
        const node = parser.parseStreamEvent({ type: "user_message", message: jektBlock({ from: "agentx", to: "agent3" }) });
        expect((node as import("./types").JektMessageNode).direction).toBe("outgoing");
    });

    test("defaults unrecognized TIER/DELIVERY values to sensitive/wan (least-trusted) rather than dropping the node", () => {
        const node = parser.parseStreamEvent({ type: "user_message", message: jektBlock({ tier: "bogus", delivery: "bogus" }) });
        const jekt = node as import("./types").JektMessageNode;
        expect(jekt.type).toBe("jekt_message");
        expect(jekt.tier).toBe("sensitive");
        expect(jekt.deliveryTier).toBe("wan");
    });

    test("does not strip message content that coincidentally matches scaffolding patterns", () => {
        const message =
            `[JEKT:FROM=agentx TO=agent3 TIER=coord DELIVERY=host TRUST=host-verified MSGID=abc123 PRIORITY=normal TS=1783386012]\n` +
            `────────────────────────────────────────────────────────────\n` +
            `From: agentx | To: agent3 | ts=1783386012\n` +
            `Reply: yes, and also ────────── here's a dash line and a From: X | Y | line in the body\n` +
            `────────────────────────────────────────────────────────────\n` +
            `Reply: bus:inject to agentx\n` +
            `[/JEKT]`;
        const node = parser.parseStreamEvent({ type: "user_message", message });
        const jekt = node as import("./types").JektMessageNode;
        expect(jekt.type).toBe("jekt_message");
        expect(jekt.message).toBe(
            "Reply: yes, and also ────────── here's a dash line and a From: X | Y | line in the body"
        );
    });

    test("falls back to a plain user_message for an unterminated jekt block", () => {
        const message = "[JEKT:FROM=agentx TO=agent3 TIER=coord]\nno closing tag here";
        const node = parser.parseStreamEvent({ type: "user_message", message });
        expect(node!.type).toBe("user_message");
        expect((node as UserMessageNode).message).toBe(message);
    });

    test("falls back to a plain user_message when FROM/TO are missing", () => {
        const message = "[JEKT:TIER=coord]\nsomething\n[/JEKT]";
        const node = parser.parseStreamEvent({ type: "user_message", message });
        expect(node!.type).toBe("user_message");
    });

    test("normal typed input starting with a bracket is not mistaken for a jekt block", () => {
        const node = parser.parseStreamEvent({ type: "user_message", message: "[not a jekt] just talking about brackets" });
        expect(node!.type).toBe("user_message");
    });
});

// ── error_result → AgentErrorNode (P1.3) ────────────────────────────────────

describe("error_result event", () => {
    test("error_result produces an agent_error node with code and message", () => {
        const p = new ClaudeCodeStreamParser();
        const node = p.parseStreamEvent({ type: "error_result", code: 401, message: "Unauthorized" });
        expect(node).not.toBeNull();
        expect(node!.type).toBe("agent_error");
        expect((node as any).code).toBe(401);
        expect((node as any).message).toBe("Unauthorized");
    });

    test("error_result with code 0 (non-HTTP error) produces agent_error node", () => {
        const p = new ClaudeCodeStreamParser();
        const node = p.parseStreamEvent({ type: "error_result", code: 0, message: "Network connection lost" });
        expect(node!.type).toBe("agent_error");
        expect((node as any).code).toBe(0);
        expect((node as any).message).toBe("Network connection lost");
    });

    test("agent_error node id is unique across consecutive error_result events", () => {
        const p = new ClaudeCodeStreamParser();
        const n1 = p.parseStreamEvent({ type: "error_result", code: 401, message: "err" });
        const n2 = p.parseStreamEvent({ type: "error_result", code: 429, message: "err" });
        expect(n1!.id).not.toBe(n2!.id);
    });

    test("error_result resets currentTextNode accumulation", () => {
        const p = new ClaudeCodeStreamParser();
        const text = p.parseStreamEvent({ type: "text", content: "hello" });
        expect(text).not.toBeNull();
        // error_result must break the text accumulation (same as other non-text events)
        const errNode = p.parseStreamEvent({ type: "error_result", code: 500, message: "oops" });
        // A new text event after the error should start a fresh node (different id)
        const text2 = p.parseStreamEvent({ type: "text", content: "world" });
        expect(text2).not.toBeNull();
        expect(text!.id).not.toBe(text2!.id);
        expect(errNode!.type).toBe("agent_error");
    });
});
