// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-doc-links.mjs against a made-up git repo: a relative Markdown link that does not resolve from the file's own
// folder fails; links that are not pointers (URLs, anchors, code, prose in parens) do not.

import { afterEach, describe, expect, it } from "vitest";
import { makeFixture } from "./lib/check-fixture.mjs";

const GATE = "check-doc-links.mjs";

let fx;
afterEach(() => fx?.remove());

function check(files, ...targets) {
    fx = makeFixture([GATE], { "docs/specs/target.md": "# t\n", ...files }).git();
    return fx.run(GATE, targets);
}

describe("check-doc-links", () => {
    it("passes when a link resolves from the folder of the file that holds it", () => {
        const r = check({ "docs/reports/a.md": "See [t](../specs/target.md#section).\n" }, "docs/reports/a.md");
        expect(r.status).toBe(0);
        expect(r.out).toContain("1 link(s) checked across 1 doc(s), 0 broken");
    });

    it("fails on a link written from the wrong folder, and names where it resolved", () => {
        const r = check({ "docs/reports/a.md": "See [t](specs/target.md).\n" }, "docs/reports/a.md");
        expect(r.status).toBe(1);
        expect(r.out).toContain("](specs/target.md)  ->  docs/reports/specs/target.md  (missing)");
    });

    it("checks a reference definition as well as an inline link", () => {
        const r = check({ "docs/reports/a.md": "[t]: specs/target.md\n" }, "docs/reports/a.md");
        expect(r.status).toBe(1);
    });

    it("skips URLs, anchors, site-absolute paths, and parenthesised prose", () => {
        const r = check(
            { "docs/a.md": "[a](https://example.com/x.md) [b](#top) [c](/internals/x/) [d](true) [e](mailto:a@b.c) [f](next)\n" },
            "docs/a.md",
        );
        expect(r.status).toBe(0);
        expect(r.out).toContain("0 link(s) checked");
    });

    it("skips links inside code spans and fenced blocks", () => {
        const r = check({ "docs/a.md": "Write `[x](gone.md)` like that.\n\n```\n[y](also-gone.md)\n```\n" }, "docs/a.md");
        expect(r.status).toBe(0);
    });

    it("does not gate on a link that leaves the repo", () => {
        const r = check({ "docs/a.md": "[m](../../outside/notes.md)\n" }, "docs/a.md");
        expect(r.status).toBe(0);
        expect(r.out).toContain("1 outside-repo (ignored)");
    });

    it("counts a repeated target once per file", () => {
        const r = check({ "docs/a.md": "[x](gone.md) and again [x](gone.md)\n" }, "docs/a.md");
        expect(r.status).toBe(1);
        expect(r.out).toContain("1 broken in 1 file(s)");
    });

    it("--all reports the backlog but never fails", () => {
        const r = check({ "docs/a.md": "[x](gone.md)\n" }, "--all");
        expect(r.status).toBe(0);
        expect(r.out).toContain("1 broken in 1 file(s)");
    });
});
