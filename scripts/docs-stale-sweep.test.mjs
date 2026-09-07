// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for docs-stale-sweep.mjs's pure parts — Status parsing, citation
// extraction/resolution, and the per-doc classification — with no git and no
// filesystem. Runs under vitest with the rest of the repo's .test.mjs files.

import { describe, expect, it } from "vitest";
import { classifyDoc, extractCitations, plausibleCitation, renderMarkdown, resolveCitation, statusOf } from "./docs-stale-sweep.mjs";

describe("docs-stale-sweep statusOf", () => {
    it("takes the first word of the first Status line, lowercased, letters only", () => {
        expect(statusOf("# T\n\n**Status:** active — Phase 0 shipped\n")).toBe("active");
        expect(statusOf("**Status:** Implemented (PR #12)\n")).toBe("implemented");
        expect(statusOf("**Status:** historical.\n**Status:** active\n")).toBe("historical");
    });

    it("is null when there is no Status line", () => {
        expect(statusOf("# T\n\nStatus: active\n")).toBeNull();
    });
});

describe("docs-stale-sweep extractCitations", () => {
    it("finds backticked and bare paths, strips line refs, dedupes", () => {
        const text = [
            "See `agentmux-srv/src/server/reactive.rs:540-606` and `runner.rs`.",
            "Also agentmux-srv/src/server/reactive.rs and (frontend/app/x.tsx).",
            "Not a file: `foo`, `Store::open`, `--verify`.",
        ].join("\n");
        expect(extractCitations(text)).toEqual(["agentmux-srv/src/server/reactive.rs", "runner.rs", "frontend/app/x.tsx"]);
    });

    it("drops the shapes that are examples rather than citations", () => {
        expect(plausibleCitation("/path/to/foo.ts")).toBe(false);
        expect(plausibleCitation(".ts/.tsx")).toBe(false);
        expect(plausibleCitation("a/.hidden.rs")).toBe(false);
        expect(plausibleCitation("agentmux-srv/src/main.rs")).toBe(true);
        expect(extractCitations("try `/workspace/foo.ts` or `.ts/.tsx` files")).toEqual([]);
    });
});

describe("docs-stale-sweep resolveCitation", () => {
    const tracked = new Set(["agentmux-srv/src/migrations/runner.rs", "agentmux-srv/src/server/mod.rs", "agentmux-cef/src/main.rs", "agentmux-srv/src/main.rs"]);
    const byBasename = new Map([
        ["runner.rs", ["agentmux-srv/src/migrations/runner.rs"]],
        ["mod.rs", ["agentmux-srv/src/server/mod.rs"]],
        ["main.rs", ["agentmux-cef/src/main.rs", "agentmux-srv/src/main.rs"]],
    ]);

    it("resolves exact paths, unique suffixes, and unique basenames", () => {
        expect(resolveCitation("agentmux-srv/src/migrations/runner.rs", tracked, byBasename)).toBe("agentmux-srv/src/migrations/runner.rs");
        expect(resolveCitation("migrations/runner.rs", tracked, byBasename)).toBe("agentmux-srv/src/migrations/runner.rs");
        expect(resolveCitation("runner.rs", tracked, byBasename)).toBe("agentmux-srv/src/migrations/runner.rs");
    });

    it("refuses to guess between an ambiguous basename, and returns null for an absent path", () => {
        expect(resolveCitation("main.rs", tracked, byBasename)).toBeNull();
        expect(resolveCitation("src/main.rs", tracked, byBasename)).toBeNull();
        expect(resolveCitation("agentmux-srv/src/gone.rs", tracked, byBasename)).toBeNull();
    });
});

describe("docs-stale-sweep classifyDoc", () => {
    const DAY = 86400;
    const now = 1_800_000_000;
    const tracked = new Set(["docs/fixtures/SPEC_X.md", "a/b/live.rs", "a/b/old.rs"]);
    const byBasename = new Map([
        ["live.rs", ["a/b/live.rs"]],
        ["old.rs", ["a/b/old.rs"]],
    ]);
    const base = { doc: "docs/fixtures/SPEC_X.md", tracked, byBasename, now, weeks: 8 };

    it("flags a stale live doc whose citation changed after it", () => {
        const touched = new Map([
            ["docs/fixtures/SPEC_X.md", now - 100 * DAY],
            ["a/b/live.rs", now - 10 * DAY],
            ["a/b/old.rs", now - 200 * DAY],
        ]);
        const r = classifyDoc({ ...base, touched, text: "**Status:** active\nsee `a/b/live.rs`, `a/b/old.rs`, `live.rs`" });
        expect(r.stale).toBe(true);
        expect(r.ageDays).toBe(100);
        // live.rs counted once although cited two ways; old.rs predates the doc.
        expect(r.drifted).toEqual([{ path: "a/b/live.rs", daysAfterDoc: 90 }]);
        expect(r.missing).toEqual([]);
        expect(r.flagged).toBe(true);
    });

    it("flags a stale live doc that cites a path which no longer exists, but not a bare unknown name", () => {
        const touched = new Map([["docs/fixtures/SPEC_X.md", now - 100 * DAY]]);
        const r = classifyDoc({ ...base, touched, text: "**Status:** proposed\n`a/b/gone.rs` and `whatever.rs`" });
        expect(r.missing).toEqual(["a/b/gone.rs"]);
        expect(r.flagged).toBe(true);
    });

    it("does not flag a recently touched doc, or a stale one whose citations are all older than it", () => {
        const drifted = new Map([
            ["docs/fixtures/SPEC_X.md", now - 10 * DAY],
            ["a/b/live.rs", now - 1 * DAY],
        ]);
        expect(classifyDoc({ ...base, touched: drifted, text: "**Status:** active\n`a/b/live.rs`" }).flagged).toBe(false);
        const quiet = new Map([
            ["docs/fixtures/SPEC_X.md", now - 100 * DAY],
            ["a/b/live.rs", now - 300 * DAY],
        ]);
        const r = classifyDoc({ ...base, touched: quiet, text: "**Status:** active\n`a/b/live.rs`" });
        expect(r.stale).toBe(true);
        expect(r.flagged).toBe(false);
    });

    it("does not judge terminal docs or docs without a Status line", () => {
        const touched = new Map([["docs/fixtures/SPEC_X.md", now - 400 * DAY]]);
        expect(classifyDoc({ ...base, touched, text: "**Status:** historical\n`a/b/gone.rs`" })).toBeNull();
        expect(classifyDoc({ ...base, touched, text: "**Status:** superseded\n`a/b/gone.rs`" })).toBeNull();
        expect(classifyDoc({ ...base, touched, text: "no status here `a/b/gone.rs`" })).toBeNull();
    });

    it("keeps a non-canonical Status word but marks it", () => {
        const touched = new Map([["docs/fixtures/SPEC_X.md", now - 100 * DAY]]);
        const r = classifyDoc({ ...base, touched, text: "**Status:** ready\n`a/b/gone.rs`" });
        expect(r.canonical).toBe(false);
        expect(r.flagged).toBe(true);
    });
});

describe("docs-stale-sweep renderMarkdown", () => {
    const report = {
        generatedAt: "2026-09-07T06:30:00.000Z",
        weeks: 8,
        shallow: false,
        counts: { docs: 10, noStatus: 2, terminal: 1, live: 7, stale: 3, flagged: 2, nonCanonicalStatus: 1 },
        flagged: [
            { doc: "docs/fixtures/A.md", status: "active", ageDays: 120, drifted: [{ path: "x/y.rs", daysAfterDoc: 30 }], missing: ["x/gone.rs"] },
            { doc: "docs/fixtures/B.md", status: "ready", ageDays: 90, drifted: [], missing: ["z/gone.rs"] },
        ],
    };

    it("renders the counts, one row per flagged doc, and honours the limit", () => {
        const md = renderMarkdown(report, 1);
        expect(md).toContain("| … **of which cite a file that changed since, or is gone** | **2** |");
        expect(md).toContain("| `docs/fixtures/A.md` | active | 120d | `x/y.rs` (+30d) | `x/gone.rs` |");
        expect(md).not.toContain("docs/fixtures/B.md");
        expect(md).toContain("… and 1 more");
    });

    it("says so when nothing is flagged, and warns on a shallow clone", () => {
        expect(renderMarkdown({ ...report, flagged: [] })).toContain("Nothing flagged.");
        expect(renderMarkdown({ ...report, shallow: true })).toContain("No git history was available");
    });
});
