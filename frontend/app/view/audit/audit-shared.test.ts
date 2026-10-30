// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { AuditEntry } from "@/types/rpc/AuditEntry";
import {
    buildRequest,
    costLabel,
    DEFAULT_KINDS,
    emptyFilter,
    entryLinks,
    isFailure,
    requestKinds,
    totalsLabel,
} from "./audit-shared";

const history = (agent_id: string, at_ms = 1000) => ({ agent_id, definition_id: `uid-${agent_id}`, at_ms });

function entry(over: Partial<AuditEntry>): AuditEntry {
    return { id: "e1", ts_ms: 1000, kind: "jekt", summary: "a → b: delivered, 10 bytes", links: {}, ...over };
}

describe("the query the filters make", () => {
    it("sends nothing for the default kinds, so srv's defaults apply", () => {
        expect(requestKinds(new Set(DEFAULT_KINDS))).toBeUndefined();
        expect(buildRequest(emptyFilter(), 5000).kinds).toBeUndefined();
    });

    it("sends the ticked kinds in the chips' order once they differ", () => {
        const kinds = new Set([...DEFAULT_KINDS, "tool"]);
        expect(requestKinds(kinds)).toEqual(["jekt", "action", "registry", "agent_start", "agent_end", "tool"]);
    });

    it("turns a range into since_ms, trims the agent and text, and adds the cursor and totals only when given", () => {
        const f = { ...emptyFilter(" korp "), range: "1h" as const, text: " refused " };
        expect(buildRequest(f, 10_000_000)).toEqual({ limit: 50, agent_id: "korp", text: "refused", since_ms: 10_000_000 - 3_600_000 });
        expect(buildRequest(emptyFilter(), 1, "c1", true)).toEqual({ limit: 50, cursor: "c1", totals: true });
    });
});

describe("a row's links", () => {
    it("gives a jekt both ends, each only with what can work", () => {
        const items = entryLinks(
            entry({
                from: { agent_id: "korp", links: { history: history("korp"), pane: { block_id: "b1" } } },
                to: { agent_id: "loap", links: { history: history("loap") } },
            })
        );
        expect(items.map((i) => i.label)).toEqual(["Sender's history", "Sender's pane", "Receiver's history"]);
    });

    it("gives an entry about one agent its own links, and none when it has none", () => {
        const start = entry({ kind: "agent_start", agent_id: "korp", links: { history: history("korp"), pane: { block_id: "b1" } } });
        expect(entryLinks(start).map((i) => i.label)).toEqual(["In history", "In its pane"]);
        expect(entryLinks(entry({ kind: "registry", links: {} }))).toEqual([]);
    });

    it("gives an action its actor's links and its target's", () => {
        const items = entryLinks(
            entry({ kind: "action", links: { history: history("korp") }, to: { agent_id: "loap", links: { pane: { block_id: "b2" } } } })
        );
        expect(items.map((i) => i.label)).toEqual(["Actor's history", "Target's pane"]);
    });
});

describe("wording", () => {
    it("marks an estimate with a ~ and a measured figure without", () => {
        expect(costLabel({ input: 100, cache_read: 0, output: 20, basis: "estimated" })).toBe("~120 tokens");
        expect(costLabel({ input: 1500, cache_read: 0, output: 500, usd: 0.5, basis: "measured" })).toBe("2.0k tokens · $0.50");
    });

    it("words the totals, estimated or not", () => {
        expect(totalsLabel({ entries: 1, input: 10, cache_read: 0, output: 0, estimated: false })).toBe("1 entry · 10 tokens");
        expect(totalsLabel({ entries: 3, input: 1000, cache_read: 0, output: 0, usd: 1.234, estimated: true })).toBe(
            "3 entries · ~1.0k tokens · ~$1.23"
        );
    });

    it("calls a row failed when srv words it so, except a registration", () => {
        expect(isFailure(entry({ summary: "a → b: failed (no pane), 10 bytes" }))).toBe(true);
        expect(isFailure(entry({ kind: "registry", summary: "failed: register" }))).toBe(false);
        expect(isFailure(entry({}))).toBe(false);
    });
});
