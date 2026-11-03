// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-docs-lifecycle.mjs against a made-up git repo: --list names the specs that break each rule, and the ratchet
// fails only for a spec this branch touched that is newly non-compliant.

import { afterEach, describe, expect, it } from "vitest";
import { makeFixture } from "./lib/check-fixture.mjs";

const GATE = "check-docs-lifecycle.mjs";
const spec = (status, extra = "") => `# S\n\n**Status:** ${status}\n${extra}`;

let fx;
afterEach(() => fx?.remove());

function repo(files) {
    fx = makeFixture([GATE], files).git();
    fx.gitRun("branch", "-M", "main");
    return fx;
}

describe("check-docs-lifecycle --list", () => {
    const files = {
        "docs/specs/ok.md": spec("implemented — shipped in #1234"),
        "docs/specs/none.md": "# No status line\n",
        "docs/specs/r1.md": spec("maybe someday"),
        "docs/specs/r2.md": spec("active — in progress"),
        "docs/specs/r3.md": spec("implemented — done"),
        "docs/specs/r4.md": spec("superseded", "**Superseded-by:** docs/specs/missing.md\n"),
        "docs/specs/archive/old.md": spec("whatever"),
    };

    it("names the offenders of each rule", () => {
        repo(files);
        const list = (rule) => fx.run(GATE, ["--list", rule]).stdout.trim();
        expect(list("noStatus")).toBe("docs/specs/none.md");
        expect(list("R1")).toContain("docs/specs/r1.md");
        expect(list("R2")).toBe("docs/specs/r2.md");
        expect(list("R3")).toBe("docs/specs/r3.md");
        expect(list("R4")).toBe("docs/specs/r4.md -> docs/specs/missing.md");
    });

    it("accepts a superseded spec whose Superseded-by resolves", () => {
        repo({ "docs/specs/new.md": spec("implemented — #1000"), "docs/specs/old.md": spec("superseded", "**Superseded-by:** new.md\n") });
        expect(fx.run(GATE, ["--list", "R4"]).stdout.trim()).toBe("");
    });

    it("reads a Status that wraps onto the next line", () => {
        repo({ "docs/specs/w.md": "# W\n\n**Status:** active — the first half is\nin #4321 and the rest remains.\n" });
        expect(fx.run(GATE, ["--list", "R2"]).stdout.trim()).toBe("");
    });

    it("rejects an unknown rule name", () => {
        repo(files);
        const r = fx.run(GATE, ["--list", "R9"]);
        expect(r.status).toBe(2);
        expect(r.out).toContain('unknown rule "R9"');
    });
});

describe("check-docs-lifecycle ratchet", () => {
    it("passes when the branch touches no specs", () => {
        repo({ "docs/specs/a.md": spec("maybe someday"), "other.txt": "1" });
        fx.gitRun("checkout", "-q", "-b", "feature");
        fx.write("other.txt", "2");
        fx.gitRun("commit", "-q", "-am", "other");
        const r = fx.run(GATE);
        expect(r.status).toBe(0);
        expect(r.out).toContain("this branch touches no specs. ok");
    });

    it("fails for a touched spec that is newly non-compliant", () => {
        repo({ "docs/specs/a.md": spec("implemented — #1234") });
        fx.gitRun("checkout", "-q", "-b", "feature");
        fx.write("docs/specs/a.md", spec("implemented — done"));
        fx.gitRun("commit", "-q", "-am", "break a");
        const r = fx.run(GATE);
        expect(r.status).toBe(1);
        expect(r.out).toContain("newly non-compliant");
        expect(r.out).toContain("docs/specs/a.md");
    });

    it("does not blame a branch for debt in a spec it did not touch", () => {
        repo({ "docs/specs/old.md": spec("maybe someday"), "docs/specs/a.md": spec("implemented — #1234") });
        fx.gitRun("checkout", "-q", "-b", "feature");
        fx.write("docs/specs/a.md", spec("implemented — #1234, plus a note"));
        fx.gitRun("commit", "-q", "-am", "touch a");
        const r = fx.run(GATE);
        expect(r.status).toBe(0);
        expect(r.out).toContain("fixed: 0   newly violating: 0");
    });

    it("counts a repaired spec as fixed", () => {
        repo({ "docs/specs/a.md": spec("implemented — done") });
        fx.gitRun("checkout", "-q", "-b", "feature");
        fx.write("docs/specs/a.md", spec("implemented — #1234"));
        fx.gitRun("commit", "-q", "-am", "fix a");
        const r = fx.run(GATE);
        expect(r.status).toBe(0);
        expect(r.out).toContain("fixed: 1   newly violating: 0");
    });
});
