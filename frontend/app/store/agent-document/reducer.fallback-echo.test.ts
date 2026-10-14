// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A fallback re-delivery's card stands for its hidden message, whose replay
// otherwise makes a "Memory reinjected" row of its own. Within one history
// page the parser pairs them; where nodes are joined (an older page, a
// snapshot, a live batch) only the reducer sees both.
// docs/specs/SPEC_CONTEXT_DELIVERY_2026_09_30.md §9 B1.

import { describe, expect, it } from "vitest";
import { composeReinjectionMessage } from "../../view/agent/memory-reinjection";
import { parseHistoryLines } from "../../view/agent/parseHistoryLines";
import type { ContextDeliveryNode, DocumentNode, MemoryReinjectionNode } from "../../view/agent/types";
import { update } from "./reducer";
import { AgentDocumentState, initialState } from "./types";

const md = (id: string): DocumentNode => ({ type: "markdown", id, content: id, timestamp: 0 });

const card = (id: string, fallback: boolean): ContextDeliveryNode => ({
    type: "context_delivery",
    id,
    reason: "resume_fresh",
    items: [{ kind: "personal_memory", name: "notes.md", sizeBytes: 2, tokens: 1 }],
    timestamp: 0,
    ...(fallback ? { fallback: true } : {}),
});

const echo = (id: string): MemoryReinjectionNode => ({
    type: "memory_reinjection",
    id,
    globalMemoryCount: 0,
    personalMemoryCount: 1,
    estimatedTokens: 1,
    perEntryTokens: [{ label: "Entry 1", source: "personal", tokens: 1, sizeBytes: 2 }],
    totalSizeBytes: { global: 0, personal: 2 },
    sizeBand: "low",
    at: 0,
});

const seed = (nodes: DocumentNode[]): AgentDocumentState => ({
    ...initialState(),
    nodes,
    nodeIdSet: new Set(nodes.map((n) => n.id)),
    nodeIndexById: new Map(nodes.map((n, i) => [n.id, i])),
});

/** The state's id maps describe exactly its nodes. */
const expectConsistent = (s: AgentDocumentState): void => {
    expect(s.nodeIdSet).toEqual(new Set(s.nodes.map((n) => n.id)));
    expect(s.nodeIndexById).toEqual(new Map(s.nodes.map((n, i) => [n.id, i])));
};

describe("a fallback card absorbs its echo row where nodes are joined", () => {
    it("an older page ending with the echo, before a document starting with the card", () => {
        const r = update(seed([card("fb", true), md("after")]), { type: "HistoryLoaded", nodes: [md("before"), echo("e")] });
        expect(r.state.nodes.map((n) => n.id)).toEqual(["before", "fb", "after"]);
        expectConsistent(r.state);
    });

    it("an older page ending with the card, before a document starting with the echo", () => {
        const r = update(seed([echo("e"), md("after")]), { type: "HistoryLoaded", nodes: [md("before"), card("fb", true)] });
        expect(r.state.nodes.map((n) => n.id)).toEqual(["before", "fb", "after"]);
        expectConsistent(r.state);
    });

    it("a snapshot restored before live nodes", () => {
        const r = update(seed([card("fb", true)]), { type: "HistoryRestored", fromSnapshot: true, nodes: [md("before"), echo("e")] });
        expect(r.state.nodes.map((n) => n.id)).toEqual(["before", "fb"]);
        expectConsistent(r.state);
    });

    it("a live card arriving after its echo", () => {
        const r = update(seed([md("before"), echo("e")]), { type: "StreamFlush", newNodes: [card("fb", true)], updatedNodes: [] });
        expect(r.state.nodes.map((n) => n.id)).toEqual(["before", "fb"]);
        expectConsistent(r.state);
    });

    it("a live echo arriving after its card, in the same batch or the next", () => {
        const same = update(seed([md("before")]), { type: "StreamFlush", newNodes: [card("fb", true), echo("e"), md("after")], updatedNodes: [] });
        expect(same.state.nodes.map((n) => n.id)).toEqual(["before", "fb", "after"]);
        expectConsistent(same.state);

        const next = update(seed([md("before"), card("fb", true)]), { type: "StreamFlush", newNodes: [echo("e")], updatedNodes: [] });
        expect(next.state.nodes.map((n) => n.id)).toEqual(["before", "fb"]);
        expectConsistent(next.state);
    });

    it("keeps a row next to a hook card: before CD2b only fallback cards stood for a hidden message", () => {
        const r = update(seed([card("hook", false)]), { type: "HistoryLoaded", nodes: [echo("e")] });
        expect(r.state.nodes.map((n) => n.id)).toEqual(["e", "hook"]);
    });

    it("keeps a row that isn't next to the card", () => {
        const r = update(seed([md("between"), card("fb", true)]), { type: "HistoryLoaded", nodes: [echo("e")] });
        expect(r.state.nodes.map((n) => n.id)).toEqual(["e", "between", "fb"]);
    });

    it("leaves a flush with no card or row alone", () => {
        const start = seed([echo("e")]);
        const r = update(start, { type: "StreamFlush", newNodes: [md("x")], updatedNodes: [] });
        expect(r.state.nodes.map((n) => n.id)).toEqual(["e", "x"]);
        expectConsistent(r.state);
    });
});

describe("two history pages split between a fallback card and its hidden message", () => {
    const fallbackFrame = JSON.stringify({
        type: "system",
        subtype: "agentmux_memory_injected",
        id: "memory-injected-fb-7",
        reason: "resume_fresh",
        fallback: true,
        delivery_id: "fb-7",
        entries: [{ label: "notes.md", source: "personal", kind: "personal_memory", name: "notes.md", size_bytes: 2, tokens: 1 }],
        timestamp: "2026-10-14T08:00:00+00:00",
    });
    const hiddenLine = JSON.stringify({
        type: "user",
        message: {
            role: "user",
            content: composeReinjectionMessage([{ label: "notes.md", source: "personal", body: "P1", sizeBytes: 2 }], "fresh_session"),
        },
        timestamp: "2026-10-14T08:00:00.500Z",
    });
    const reply = JSON.stringify({ type: "assistant", message: { id: "m1", role: "assistant", content: [{ type: "text", text: "Noted." }] } });
    const user = JSON.stringify({ type: "user", message: { role: "user", content: "next question" } });

    const load = (older: string[], newer: string[]): DocumentNode[] => {
        // Newest page first, as the pane loads them.
        const first = update(initialState(), { type: "HistoryLoaded", nodes: parseHistoryLines(newer, "claude-stream-json").nodes });
        return update(first.state, { type: "HistoryLoaded", nodes: parseHistoryLines(older, "claude-stream-json").nodes }).state.nodes;
    };

    it("shows one card when the card is on the older page", () => {
        const nodes = load([fallbackFrame], [hiddenLine, reply, user]);
        expect(nodes.filter((n) => n.type === "context_delivery" || n.type === "memory_reinjection").map((n) => n.type)).toEqual(["context_delivery"]);
    });

    it("shows one card when the hidden message is on the older page", () => {
        const nodes = load([hiddenLine, reply], [fallbackFrame, user]);
        expect(nodes.filter((n) => n.type === "context_delivery" || n.type === "memory_reinjection").map((n) => n.type)).toEqual(["context_delivery"]);
    });
});
