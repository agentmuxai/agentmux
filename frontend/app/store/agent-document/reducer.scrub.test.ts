// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The scrub of orphaned in-progress nodes (`scrubOrphanedInProgress`, reached through the
 * `ScrubOrphanedInProgress`, `SessionEnd`, `HistoryLoaded` and `HistoryRestored` commands),
 * beyond reducer.test.ts: a running shell, several nodes scrubbed in one pass, what a
 * scrubbed node keeps, and the edges of an older page that is merged in front of other nodes.
 */

import { describe, expect, it } from "vitest";
import type { DocumentNode, ShellNode, ToolNode } from "../../agent-model/types";
import { update } from "./reducer";
import { AgentDocumentState, initialState, ReducerResult } from "./types";

const md = (id: string, content = id): DocumentNode => ({ type: "markdown", id, content, timestamp: 0 });

const thinking = (id: string, metadata: Record<string, unknown> = {}): DocumentNode => ({
    type: "markdown",
    id,
    content: `${id} content`,
    timestamp: 0,
    metadata: { thinking: true, ...metadata },
});

const tool = (id: string, over: Partial<ToolNode> = {}): ToolNode => ({
    type: "tool",
    id,
    tool: "Bash",
    params: { command: "echo hi" },
    status: "running",
    collapsed: false,
    summary: "🔧 Bash echo hi",
    ...over,
});

const question = (id: string, over: Partial<ToolNode> = {}): ToolNode =>
    tool(id, {
        status: "awaiting_answer",
        question: { type: "ask_user_question", tool_use_id: id, questions: [] },
        summary: "❓ Waiting for your answer",
        ...over,
    });

const shell = (id: string, over: Partial<ShellNode> = {}): ShellNode => ({
    type: "shell",
    id,
    cmd: "npm run dev",
    title: "dev server",
    cwd: "/work/app",
    status: "running",
    spawnedAt: 100,
    log: { chunks: [{ kind: "stdout", content: "ready", timestamp: 150 }], open: true },
    ...over,
});

const seed = (nodes: DocumentNode[]): AgentDocumentState => ({
    ...initialState(),
    nodes,
    nodeIdSet: new Set(nodes.map((n) => n.id)),
    nodeIndexById: new Map(nodes.map((n, i) => [n.id, i])),
});

const scrub = (nodes: DocumentNode[], at = 9999, scope?: "tools-only") =>
    update(seed(nodes), { type: "ScrubOrphanedInProgress", at, scope });

type Scrubbed = Extract<ReducerResult["events"][number], { type: "orphans-scrubbed" }>;

const scrubbedEvent = (events: ReducerResult["events"]): Scrubbed | undefined =>
    events.find((e): e is Scrubbed => e.type === "orphans-scrubbed");

describe("a running shell", () => {
    it("is stopped, with the time of the scrub and its log closed", () => {
        const r = scrub([shell("dev")], 4242);

        const s = r.state.nodes[0] as ShellNode;
        expect(s.status).toBe("stopped");
        expect(s.exitedAt).toBe(4242);
        expect(s.log.open).toBe(false);
    });

    it("keeps everything else about it, including what its log printed", () => {
        const r = scrub([shell("dev", { exitCode: undefined })]);

        expect(r.state.nodes[0]).toEqual({
            type: "shell",
            id: "dev",
            cmd: "npm run dev",
            title: "dev server",
            cwd: "/work/app",
            status: "stopped",
            exitCode: undefined,
            spawnedAt: 100,
            exitedAt: 9999,
            log: { chunks: [{ kind: "stdout", content: "ready", timestamp: 150 }], open: false },
        });
    });

    it("counts with the stopped tools, but is not reported as a resolved tool", () => {
        const r = scrub([shell("dev")]);

        expect(r.events).toEqual([
            { type: "orphans-scrubbed", markdownCanceled: 0, toolsCanceled: 1, resolvedToolNodes: [] },
        ]);
    });

    it("is counted next to a running tool, which is the only one reported", () => {
        const r = scrub([tool("t1"), shell("dev")]);

        expect(r.events).toEqual([
            {
                type: "orphans-scrubbed",
                markdownCanceled: 0,
                toolsCanceled: 2,
                resolvedToolNodes: [{ id: "t1", status: "canceled", toolName: "Bash" }],
            },
        ]);
    });

    it.each(["exited-ok", "exited-err", "stopped"] as const)("is left alone once it is %s", (status) => {
        const s0 = seed([shell("dev", { status, exitedAt: 120, log: { chunks: [], open: false } })]);

        const r = update(s0, { type: "ScrubOrphanedInProgress", at: 9999 });

        expect(r.state).toBe(s0);
        expect(r.events).toEqual([]);
    });

    it("is left running by a tools-only pass", () => {
        const s0 = seed([shell("dev")]);

        const r = update(s0, { type: "ScrubOrphanedInProgress", at: 9999, scope: "tools-only" });

        expect(r.state).toBe(s0);
    });

    it("is stopped when a session ends", () => {
        const r = update(seed([md("m1"), shell("dev")]), { type: "SessionEnd", at: 7000 });

        expect((r.state.nodes[1] as ShellNode).status).toBe("stopped");
        expect((r.state.nodes[1] as ShellNode).exitedAt).toBe(7000);
        expect(scrubbedEvent(r.events)).toMatchObject({ toolsCanceled: 1 });
    });

    it("is stopped when a snapshot with it running is restored", () => {
        const r = update(initialState(), { type: "HistoryRestored", fromSnapshot: true, nodes: [shell("dev")] });

        expect((r.state.nodes[0] as ShellNode).status).toBe("stopped");
    });

    it("is stopped in the middle of the document too", () => {
        const r = scrub([md("a"), shell("dev"), md("b")]);

        expect((r.state.nodes[1] as ShellNode).status).toBe("stopped");
        expect(r.state.nodes[0]).toEqual(md("a"));
        expect(r.state.nodes[2]).toEqual(md("b"));
    });
});

describe("several nodes scrubbed in one pass", () => {
    it("cancels every running tool, not only the last, and reports them in order", () => {
        const r = scrub([tool("t1"), md("m"), tool("t2")]);

        expect((r.state.nodes[0] as ToolNode).status).toBe("canceled");
        expect((r.state.nodes[2] as ToolNode).status).toBe("canceled");
        expect(scrubbedEvent(r.events)).toEqual({
            type: "orphans-scrubbed",
            markdownCanceled: 0,
            toolsCanceled: 2,
            resolvedToolNodes: [
                { id: "t1", status: "canceled", toolName: "Bash" },
                { id: "t2", status: "canceled", toolName: "Bash" },
            ],
        });
    });

    it("keeps a canceled tool canceled when an answered question is resolved after it", () => {
        const r = scrub([tool("t1"), question("q1"), md("after")]);

        expect((r.state.nodes[0] as ToolNode).status).toBe("canceled");
        expect((r.state.nodes[1] as ToolNode).status).toBe("success");
    });

    it("keeps a resolved question when a shell is stopped after it", () => {
        const r = scrub([question("q1"), shell("dev"), md("after")]);

        expect((r.state.nodes[0] as ToolNode).status).toBe("success");
        expect((r.state.nodes[1] as ShellNode).status).toBe("stopped");
    });

    it("does everything at once: a tool, a question, a shell and the thinking tail", () => {
        const r = scrub([tool("t1"), question("q1"), shell("dev"), md("m"), thinking("tail")], 5);

        expect(r.state.nodes.map((n) => (n as { status?: string }).status)).toEqual([
            "canceled",
            "success",
            "stopped",
            undefined,
            undefined,
        ]);
        expect((r.state.nodes[4] as { metadata: Record<string, unknown> }).metadata).toEqual({
            thinking: false,
            canceled: true,
            canceledAt: 5,
        });
        expect(scrubbedEvent(r.events)).toEqual({
            type: "orphans-scrubbed",
            markdownCanceled: 1,
            toolsCanceled: 3,
            resolvedToolNodes: [
                { id: "t1", status: "canceled", toolName: "Bash" },
                { id: "q1", status: "success", toolName: "Bash" },
            ],
        });
    });

    it("leaves the nodes it did not change as they were", () => {
        const keep = md("keep");
        const done = tool("done", { status: "success" });
        const r = scrub([keep, tool("t1"), done]);

        expect(r.state.nodes[0]).toBe(keep);
        expect(r.state.nodes[2]).toBe(done);
    });

    it("leaves the document it was given as it was", () => {
        const before = seed([tool("t1"), shell("dev"), thinking("th")]);
        const nodes = before.nodes.slice();

        const r = update(before, { type: "ScrubOrphanedInProgress", at: 9999 });

        expect(r.state.nodes).not.toBe(before.nodes);
        expect(before.nodes).toEqual(nodes);
        expect(before.nodes.every((n, i) => n === nodes[i])).toBe(true);
    });
});

describe("what a scrubbed node keeps", () => {
    it("a thinking tail keeps its content and its other metadata", () => {
        const r = scrub([md("m"), thinking("t", { model: "opus", seconds: 12 })], 321);

        expect(r.state.nodes[1]).toEqual({
            type: "markdown",
            id: "t",
            content: "t content",
            timestamp: 0,
            metadata: { thinking: false, canceled: true, canceledAt: 321, model: "opus", seconds: 12 },
        });
    });

    it("a running tool with no log is left with none", () => {
        const r = scrub([tool("t1")]);

        const t = r.state.nodes[0] as ToolNode;
        expect(t.status).toBe("canceled");
        expect(t.log).toBeUndefined();
    });

    it("a running tool keeps the chunks its log had, closed", () => {
        const withLog: ToolNode = {
            ...tool("t1"),
            log: { chunks: [{ kind: "stdout", content: "half", timestamp: 1 }], open: true },
        };

        const r = scrub([withLog]);

        expect((r.state.nodes[0] as ToolNode).log).toEqual({
            chunks: [{ kind: "stdout", content: "half", timestamp: 1 }],
            open: false,
        });
    });

    it("a resolved question keeps everything but its status, question and summary", () => {
        const r = scrub([question("q1", { params: { command: "ask" } }), md("after")]);

        expect(r.state.nodes[0]).toEqual({
            type: "tool",
            id: "q1",
            tool: "Bash",
            params: { command: "ask" },
            status: "success",
            collapsed: false,
            question: undefined,
            summary: "❓ Question answered",
        });
    });

    it("a tool is reported by its toolName when it has one, else by its tool", () => {
        const r = scrub([
            tool("named", { tool: "Other", toolName: "Bash" }),
            tool("plain", { tool: "Read" }),
            question("q-named", { tool: "Other", toolName: "AskUserQuestion" }),
            question("q-plain", { tool: "Other" }),
            md("after"),
        ]);

        expect(scrubbedEvent(r.events)?.resolvedToolNodes).toEqual([
            { id: "named", status: "canceled", toolName: "Bash" },
            { id: "plain", status: "canceled", toolName: "Read" },
            { id: "q-named", status: "success", toolName: "AskUserQuestion" },
            { id: "q-plain", status: "success", toolName: "Other" },
        ]);
    });

    it("an empty document has nothing to scrub", () => {
        const s0 = seed([]);

        const r = update(s0, { type: "ScrubOrphanedInProgress", at: 1 });

        expect(r.state).toBe(s0);
        expect(r.events).toEqual([]);
    });

    it("a thinking node that is not the tail keeps thinking, whatever follows it", () => {
        const s0 = seed([thinking("t"), shell("done", { status: "exited-ok" })]);

        const r = update(s0, { type: "ScrubOrphanedInProgress", at: 1 });

        expect(r.state).toBe(s0);
    });

    it("a lone thinking node is its own tail", () => {
        const r = scrub([thinking("t")], 8);

        expect((r.state.nodes[0] as { metadata: Record<string, unknown> }).metadata).toMatchObject({
            thinking: false,
            canceled: true,
            canceledAt: 8,
        });
    });

    it("a tail question that is the only node is left answerable", () => {
        const s0 = seed([question("q1")]);

        const r = update(s0, { type: "ScrubOrphanedInProgress", at: 1 });

        expect(r.state).toBe(s0);
    });
});

describe("an older page merged in front of nodes already in the document", () => {
    // useHistoryPagination.loadOlder: the page's last node is not the document's tail.
    const olderPage = (existing: DocumentNode[], page: DocumentNode[]) =>
        update(seed(existing), { type: "HistoryLoaded", nodes: page });

    it("leaves a single thinking node alone when one node is already there", () => {
        const r = olderPage([md("live")], [thinking("old-think")]);

        expect((r.state.nodes[0] as { metadata: Record<string, unknown> }).metadata).toEqual({ thinking: true });
        expect(scrubbedEvent(r.events)).toBeUndefined();
    });

    it("leaves a thinking node at the start of the page alone", () => {
        const r = olderPage([md("live")], [thinking("old-think"), md("old-text")]);

        expect((r.state.nodes[0] as { metadata: Record<string, unknown> }).metadata).toEqual({ thinking: true });
        expect(scrubbedEvent(r.events)).toBeUndefined();
    });

    it("resolves a question that ends the page: the conversation continued past it", () => {
        const r = olderPage([md("live")], [question("old-q")]);

        const q = r.state.nodes[0] as ToolNode;
        expect(q.status).toBe("success");
        expect(q.question).toBeUndefined();
        expect(scrubbedEvent(r.events)).toEqual({
            type: "orphans-scrubbed",
            markdownCanceled: 0,
            toolsCanceled: 1,
            resolvedToolNodes: [{ id: "old-q", status: "success", toolName: "Bash" }],
        });
    });

    it("resolves a question at the start of the page", () => {
        const r = olderPage([md("live")], [question("old-q"), md("old-text")]);

        expect((r.state.nodes[0] as ToolNode).status).toBe("success");
    });

    it("still stops a running shell in the page", () => {
        const r = olderPage([md("live")], [shell("old-dev")]);

        expect((r.state.nodes[0] as ShellNode).status).toBe("stopped");
    });

    it("scrubs a thinking tail when the document was empty: the page is the whole document", () => {
        const r = update(initialState(), { type: "HistoryLoaded", nodes: [md("a"), thinking("t")] });

        expect((r.state.nodes[1] as { metadata: Record<string, unknown> }).metadata).toMatchObject({
            thinking: false,
            canceled: true,
        });
    });

    it("scrubs the tail of a restored snapshot even when live nodes are already there", () => {
        const r = update(seed([md("live")]), {
            type: "HistoryRestored",
            fromSnapshot: true,
            nodes: [md("a"), thinking("snap-think")],
        });

        const t = r.state.nodes.find((n) => n.id === "snap-think") as { metadata: Record<string, unknown> };
        expect(t.metadata).toMatchObject({ thinking: false, canceled: true });
    });

    it("leaves a question that ends a restored snapshot answerable", () => {
        const r = update(seed([md("live")]), {
            type: "HistoryRestored",
            fromSnapshot: true,
            nodes: [md("a"), question("q1")],
        });

        const q = r.state.nodes.find((n) => n.id === "q1") as ToolNode;
        expect(q.status).toBe("awaiting_answer");
    });
});

describe("a session that ends with a scrub", () => {
    it("SessionEnd scrubs the tail question's neighbours but leaves the tail question", () => {
        const r = update(seed([tool("t1"), question("q-tail")]), { type: "SessionEnd", at: 9 });

        expect((r.state.nodes[0] as ToolNode).status).toBe("canceled");
        expect((r.state.nodes[1] as ToolNode).status).toBe("awaiting_answer");
    });

    it("SessionEnd, unlike a tools-only pass, also cancels the thinking tail", () => {
        const r = update(seed([thinking("t")]), { type: "SessionEnd", at: 9 });

        expect((r.state.nodes[0] as { metadata: Record<string, unknown> }).metadata).toMatchObject({ canceled: true });
    });
});
