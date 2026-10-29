// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Swarm filter's matching. docs/specs/SPEC_SWARM_FILTER_BAR_2026_10_29.md §2, §3. */

import { describe, expect, it } from "vitest";
import {
    FilterMembership,
    isFiltering,
    nodeMatches,
    parseQuery,
    remoteMatches,
    splitMatches,
    stoppedMatches,
    workTextsOf,
} from "./swarm-filter";
import type { AgentTreeNode } from "./swarm-model";
import type { RemoteRow } from "./swarm-remote";

const node = (over: Partial<AgentTreeNode> = {}): AgentTreeNode =>
    ({
        blockId: "b1",
        agentName: "Lark",
        line: { text: "Fixing the login redirect", source: "generated" },
        currentTool: null,
        todoRows: [],
        agentToolRows: [],
        workflowRows: [],
        shellRows: [],
        cronRows: [],
        ...over,
    }) as unknown as AgentTreeNode;
const remote = (name: string, line?: string): RemoteRow => ({ section: {} as never, agent: { name, block_id: "r1", line } as never });

describe("parseQuery", () => {
    it("is no filter for nothing, blanks and a bare prefix", () => {
        for (const raw of [undefined, null, "", "   ", "name:", "work:  "]) expect(isFiltering(parseQuery(raw))).toBe(false);
    });

    it("splits words and lower-cases them; no prefix searches both", () => {
        expect(parseQuery("  Fix   LOGIN ")).toEqual({ words: ["fix", "login"], scope: "both" });
    });

    it("a first word name: or work: scopes the match, in any case, with or without a space after it", () => {
        expect(parseQuery("name:lark")).toEqual({ words: ["lark"], scope: "name" });
        expect(parseQuery("NAME: lark camper")).toEqual({ words: ["lark", "camper"], scope: "name" });
        expect(parseQuery("work:login")).toEqual({ words: ["login"], scope: "work" });
    });

    it("a prefix not at the start is just text", () => {
        expect(parseQuery("fix name:lark")).toEqual({ words: ["fix", "name:lark"], scope: "both" });
    });
});

describe("nodeMatches", () => {
    it("matches the name, and the work, case-insensitively, as substrings", () => {
        expect(nodeMatches(parseQuery("lar"), node())).toBe(true);
        expect(nodeMatches(parseQuery("LOGIN"), node())).toBe(true);
        expect(nodeMatches(parseQuery("deploy"), node())).toBe(false);
    });

    it("every word must be found, in the name or the work", () => {
        expect(nodeMatches(parseQuery("lark login"), node())).toBe(true);
        expect(nodeMatches(parseQuery("lark deploy"), node())).toBe(false);
    });

    it("name: reads the name only, work: the work only", () => {
        expect(nodeMatches(parseQuery("name:login"), node())).toBe(false);
        expect(nodeMatches(parseQuery("name:lark"), node())).toBe(true);
        expect(nodeMatches(parseQuery("work:lark"), node())).toBe(false);
        expect(nodeMatches(parseQuery("work:login"), node())).toBe(true);
    });

    it("reads the in-progress todo (not a finished or waiting one), the tool in flight and every sub-row's label", () => {
        const n = node({
            currentTool: "Bash",
            todoRows: [
                { text: "wire the api", status: "in_progress" },
                { text: "old chore", status: "completed" },
                { text: "later chore", status: "pending" },
            ],
            agentToolRows: [{ display_name: "Reviewer", slug: "rev-1" }, { display_name: null, slug: "scout-7" }],
            workflowRows: [{ name: "Release train" }],
            shellRows: [{ title: "npm run dev" }],
            cronRows: [{ name: "nightly sweep" }],
        } as unknown as Partial<AgentTreeNode>);
        for (const hit of ["bash", "wire", "reviewer", "scout-7", "release", "npm run", "nightly"]) {
            expect(nodeMatches(parseQuery(`work:${hit}`), n), hit).toBe(true);
        }
        for (const miss of ["old chore", "later chore", "rev-1"]) expect(nodeMatches(parseQuery(`work:${miss}`), n), miss).toBe(false);
    });

    it("an empty query matches everything", () => {
        expect(nodeMatches(parseQuery(""), node())).toBe(true);
    });

    it("workTextsOf has no empty entries", () => {
        expect(workTextsOf(node({ currentTool: null }))).toEqual(["Fixing the login redirect"]);
    });
});

describe("remote and stopped rows", () => {
    it("a remote agent matches on its name and the line its instance shares", () => {
        expect(remoteMatches(parseQuery("bob"), remote("Bob", "on the build"))).toBe(true);
        expect(remoteMatches(parseQuery("build"), remote("Bob", "on the build"))).toBe(true);
        expect(remoteMatches(parseQuery("name:build"), remote("Bob", "on the build"))).toBe(false);
        expect(remoteMatches(parseQuery("zzz"), remote("Bob"))).toBe(false);
    });

    it("a stopped agent matches on its name only: work: shows none", () => {
        expect(stoppedMatches(parseQuery("lark"), { name: "Lark" })).toBe(true);
        expect(stoppedMatches(parseQuery("name:lar"), { name: "Lark" })).toBe(true);
        expect(stoppedMatches(parseQuery("work:lark"), { name: "Lark" })).toBe(false);
    });
});

describe("splitMatches", () => {
    const join = (parts: { text: string }[]) => parts.map((p) => p.text).join("");

    it("marks every occurrence of every word, case-insensitively, and the pieces join back to the text", () => {
        const parts = splitMatches("Fixing the Login redirect login", ["login", "fix"]);
        expect(parts.filter((p) => p.match).map((p) => p.text)).toEqual(["Fix", "Login", "login"]);
        expect(join(parts)).toBe("Fixing the Login redirect login");
    });

    it("merges overlapping words into one mark", () => {
        const parts = splitMatches("authentication", ["auth", "thent"]);
        expect(parts).toEqual([
            { text: "authent", match: true },
            { text: "ication", match: false },
        ]);
    });

    it("is the text whole when there is nothing to mark", () => {
        expect(splitMatches("abc", [])).toEqual([{ text: "abc", match: false }]);
        expect(splitMatches("abc", ["x"])).toEqual([{ text: "abc", match: false }]);
        expect(splitMatches("", ["x"])).toEqual([{ text: "", match: false }]);
    });
});

describe("FilterMembership: a list never changes under the cursor", () => {
    const keys = ["a", "b", "c"];

    it("shows the rows that match, in order, and everything with no filter", () => {
        const m = new FilterMembership();
        expect(m.update(parseQuery(""), keys, () => false)).toEqual(keys);
        expect(m.update(parseQuery("x"), keys, (k) => k !== "b")).toEqual(["a", "c"]);
    });

    it("a row that stops matching on its own update stays until the filter text changes", () => {
        const m = new FilterMembership();
        const q = parseQuery("login");
        const matching = new Set(["a", "b"]);
        expect(m.update(q, keys, (k) => matching.has(k))).toEqual(["a", "b"]);
        matching.delete("b"); // b's own line changed
        expect(m.update(q, keys, (k) => matching.has(k))).toEqual(["a", "b"]);
        // The person types: every row is judged again.
        expect(m.update(parseQuery("login x"), keys, (k) => matching.has(k))).toEqual(["a"]);
    });

    it("a row that starts matching appears at once", () => {
        const m = new FilterMembership();
        const q = parseQuery("login");
        const matching = new Set(["a"]);
        expect(m.update(q, keys, (k) => matching.has(k))).toEqual(["a"]);
        matching.add("c");
        expect(m.update(q, keys, (k) => matching.has(k))).toEqual(["a", "c"]);
    });

    it("clearing the filter forgets what was held, and a row that left the list is gone", () => {
        const m = new FilterMembership();
        const q = parseQuery("login");
        m.update(q, keys, (k) => k === "a");
        m.update(parseQuery(""), keys, () => false);
        expect(m.update(q, keys, (k) => k === "c")).toEqual(["c"]);
        expect(m.update(q, ["a", "c"], (k) => k === "c")).toEqual(["c"]);
    });
});
