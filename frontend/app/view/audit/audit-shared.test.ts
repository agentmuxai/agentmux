// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { AuditEntry } from "@/types/rpc/AuditEntry";
import {
    buildRequest,
    costLabel,
    countNew,
    DEFAULT_KINDS,
    emptyFilter,
    entryLinks,
    isFailure,
    newPillLabel,
    requestKinds,
    reuseRows,
    sectionsOf,
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
        expect(buildRequest(f, 10_000_000)).toEqual({ limit: 50, agent_id: "korp", text: "refused", since_ms: 6_360_000, scope: "channel" });
        expect(buildRequest(emptyFilter(), 1, "c1", true)).toEqual({ limit: 50, cursor: "c1", totals: true, scope: "channel" });
    });
});

describe("a relative range", () => {
    it("starts at the same minute for every poll within it, so srv's kept totals are found again", () => {
        const f = { ...emptyFilter(), range: "24h" as const };
        const a = buildRequest(f, 100_020_000 + 1_000).since_ms;
        const b = buildRequest(f, 100_020_000 + 30_000).since_ms;
        expect(a).toBe(b);
        expect(a! % 60_000).toBe(0);
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


describe("what is new since the list was read", () => {
    it("counts the newest page's entries the list does not hold", () => {
        const shown = [entry({ id: "a" }), entry({ id: "b" })];
        expect(countNew(shown, [entry({ id: "c" }), entry({ id: "a" }), entry({ id: "b" })])).toBe(1);
        expect(countNew(shown, shown)).toBe(0);
        expect(countNew(shown, [])).toBe(0);
    });

    it("says 'at least' for a full page, since the rest is not asked for", () => {
        expect(newPillLabel(3)).toBe("3 new");
        expect(newPillLabel(50)).toBe("50+ new");
    });

    it("keeps the object of a row that did not change, and takes the one that did", () => {
        const a = entry({ id: "a" });
        const b = entry({ id: "b", summary: "sent" });
        const next = reuseRows([a, b], [entry({ id: "n" }), { ...a }, { ...b, summary: "delivered" }]);
        expect(next[1]).toBe(a);
        expect(next[2].summary).toBe("delivered");
        expect(next[0].id).toBe("n");
    });
});

describe("the scope", () => {
    it("is this channel's entries until it is changed, and goes in the query", () => {
        expect(emptyFilter().scope).toBe("channel");
        expect(buildRequest(emptyFilter(), 5000).scope).toBe("channel");
        expect(buildRequest({ ...emptyFilter(), scope: "machine" }, 5000).scope).toBe("machine");
    });
});

describe("sections", () => {
    const from = (id: string, channel?: string, extra = {}) => entry({ id, provenance: channel === undefined ? undefined : { channel, ...extra } });

    it("cut the list into one section per channel, in the order each first appears, keeping the list's order within one", () => {
        const sections = sectionsOf([from("a", "stable"), from("b", "dev"), from("c", "stable"), from("d", "dev")], "dev");
        expect(sections.map((s) => [s.key, s.entries.map((e) => e.id)])).toEqual([
            ["stable", ["a", "c"]],
            ["dev", ["b", "d"]],
        ]);
    });

    it("give an entry that names no channel to the channel that answered", () => {
        const sections = sectionsOf([from("a"), from("b", "dev"), from("c", "stable")], "dev");
        expect(sections.map((s) => [s.key, s.entries.map((e) => e.id)])).toEqual([
            ["dev", ["a", "b"]],
            ["stable", ["c"]],
        ]);
    });

    it("are labelled host, channel and version as far as the entries say", () => {
        const [full, bare] = sectionsOf([from("a", "stable", { host: "narko", version: "0.60.19" }), from("b", "dev")], undefined);
        expect(full.label).toBe("narko · stable · 0.60.19");
        expect(bare.label).toBe("dev");
        expect(sectionsOf([from("c")], undefined)[0].label).toBe("this channel");
        // A channel with no name is left out of the label, not named "this channel" between a host and a version.
        expect(sectionsOf([from("d", "", { host: "starpower", version: "0.60.19" })], undefined)[0].label).toBe("starpower · 0.60.19");
    });
});
