// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { mergesOf, pickBuilds, planMarkdown } from "./lib/nightlyplan.mjs";

const run = (id, sha, createdAt, portable = true) => ({ id, sha, createdAt, portable });

describe("which two nightly builds are compared", () => {
    const runs = [
        run(5, "eee", "2026-10-19T12:00:00Z", true), // today: the run was red (macOS) but the portable is there
        run(4, "ddd", "2026-10-18T10:00:00Z"),
        run(3, "ccc", "2026-10-17T10:00:00Z"),
    ];

    it("takes the newest with a live portable as B and the one before as A, whatever the run's own conclusion", () => {
        const p = pickBuilds(runs);
        expect([p.b.id, p.a.id]).toEqual([5, 4]);
    });

    it("skips a run whose portable has expired", () => {
        const p = pickBuilds([run(5, "eee", "2026-10-19T12:00:00Z", false), run(4, "ddd", "2026-10-18T10:00:00Z"), run(3, "ccc", "2026-10-17T10:00:00Z")]);
        expect([p.b.id, p.a.id]).toEqual([4, 3]);
    });

    it("skips nights with the same commit as B: a build is not compared with itself", () => {
        const p = pickBuilds([run(5, "eee", "2026-10-19T12:00:00Z"), run(4, "eee", "2026-10-18T10:00:00Z"), run(3, "ccc", "2026-10-17T10:00:00Z")]);
        expect([p.b.id, p.a.id]).toEqual([5, 3]);
        const same = pickBuilds([run(5, "eee", "2026-10-19T12:00:00Z"), run(4, "eee", "2026-10-18T10:00:00Z")]);
        expect(same.a).toBeNull();
        expect(same.reason).toMatch(/same commit/);
    });

    it("measures B alone when nothing older is left", () => {
        const p = pickBuilds([run(5, "eee", "2026-10-19T12:00:00Z")]);
        expect(p.a).toBeNull();
        expect(p.b.id).toBe(5);
        expect(p.reason).toMatch(/alone/);
    });

    it("honours a chosen B and compares it with what came before it, not with a newer run", () => {
        const p = pickBuilds(runs, { bRunId: "4" });
        expect([p.b.id, p.a.id]).toEqual([4, 3]);
        expect(pickBuilds(runs, { bRunId: 99 }).b).toBeNull();
        expect(pickBuilds([]).b).toBeNull();
    });
});

describe("what was merged between the builds", () => {
    const commit = (sha, message) => ({ sha, commit: { message } });

    it("lists the newest first, one line each, and says how many there were", () => {
        const m = mergesOf({ commits: [commit("a".repeat(40), "old\n\nbody"), commit("b".repeat(40), "new")], total_commits: 2 });
        expect(m.total).toBe(2);
        expect(m.truncated).toBe(false);
        expect(m.shown.map((c) => [c.sha, c.title])).toEqual([["bbbbbbbbb", "new"], ["aaaaaaaaa", "old"]]);
    });

    it("caps a long list and says so", () => {
        const commits = Array.from({ length: 60 }, (_, i) => commit(String(i).padStart(40, "0"), `c${i}`));
        const m = mergesOf({ commits, total_commits: 60 });
        expect(m.shown).toHaveLength(40);
        expect(m.shown[0].title).toBe("c59");
        expect(m.truncated).toBe(true);
    });

    it("renders the page head", () => {
        const a = run(4, "d".repeat(40), "2026-10-18T10:00:00Z");
        const b = run(5, "e".repeat(40), "2026-10-19T12:00:00Z");
        const md = planMarkdown({ a, b, merges: mergesOf({ commits: [commit("f".repeat(40), "fix: x (#1)")] }) });
        expect(md).toContain("**B (under test):** run 5, commit `eeeeeeeee`, built 2026-10-19");
        expect(md).toContain("**A (compared with):** run 4");
        expect(md).toContain("Merged between A and B (1):");
        expect(md).toContain("- `fffffffff` fix: x (#1)");
        expect(planMarkdown({ a: null, b, merges: null, reason: "no older nightly still has its portable: B is measured alone" })).toContain("**A:** none");
        expect(planMarkdown({ a, b, merges: { total: 0, shown: [], truncated: false } })).toContain("Nothing was merged");
    });
});
