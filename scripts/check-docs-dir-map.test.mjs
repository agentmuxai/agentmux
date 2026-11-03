// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-docs-dir-map.mjs against a made-up git repo: docs/README.md's table must name exactly the docs/
// directories that hold tracked files.

import { afterEach, describe, expect, it } from "vitest";
import { makeFixture } from "./lib/check-fixture.mjs";

let fx;
afterEach(() => fx?.remove());

function run(readme, files) {
    fx = makeFixture(["check-docs-dir-map.mjs"], { "docs/README.md": readme, ...files }).git();
    return fx.run("check-docs-dir-map.mjs");
}

const TABLE = "| Where | What |\n|---|---|\n| `specs/` | specs |\n| `reports/` | reports |\n";

describe("check-docs-dir-map", () => {
    it("passes when the table and the directories agree", () => {
        const r = run(TABLE, { "docs/specs/a.md": "", "docs/reports/b.md": "" });
        expect(r.status).toBe(0);
        expect(r.out).toContain("2 directories, all listed and all real");
    });

    it("fails on a directory the table names but that holds no tracked file", () => {
        const r = run(TABLE, { "docs/specs/a.md": "" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("claimed but empty/absent   docs/reports/");
    });

    it("fails on a directory with tracked files that the table does not name", () => {
        const r = run(TABLE, { "docs/specs/a.md": "", "docs/reports/b.md": "", "docs/debug/c.md": "" });
        expect(r.status).toBe(1);
        expect(r.out).toContain("exists but not in the map  docs/debug/");
    });

    it("reads table rows only, not prose that mentions a directory", () => {
        const r = run(`${TABLE}\nThere is no \`docs-internal/\` directory.\n`, { "docs/specs/a.md": "", "docs/reports/b.md": "" });
        expect(r.status).toBe(0);
    });

    it("does not count files directly under docs/", () => {
        const r = run(TABLE, { "docs/specs/a.md": "", "docs/reports/b.md": "", "docs/NOTES.md": "" });
        expect(r.status).toBe(0);
    });
});
