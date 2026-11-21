// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { GATES, HEAVY_GATES, NOT_RUN } from "./lib/preflight-gates.mjs";
import { changesetVerdict, heavyWanted, parseArgs, reasonOf, unknownIds } from "./preflight.mjs";

const repo = join(dirname(fileURLToPath(import.meta.url)), "..");

/** The scripts the `docs` job of ci-pr.yml runs: every `node|bash <script>` on a step line, comments left out. */
function docsJobScripts() {
    const lines = readFileSync(join(repo, ".github/workflows/ci-pr.yml"), "utf8").split(/\r?\n/);
    const start = lines.findIndex((l) => /^ {2}docs:\s*$/.test(l));
    expect(start).toBeGreaterThan(-1);
    const end = lines.findIndex((l, i) => i > start && /^ {2}[A-Za-z0-9_-]+:\s*$/.test(l));
    const scripts = [];
    for (const l of lines.slice(start, end < 0 ? undefined : end)) {
        const m = l.match(/^\s*(?:run:\s*)?(?:node|bash)\s+(\S+\.(?:mjs|sh))/);
        if (m && !l.trim().startsWith("#")) scripts.push(m[1]);
    }
    return scripts;
}

describe("preflight gates", () => {
    it("covers every script the docs job of ci-pr.yml runs, or says why not", () => {
        const known = new Set([...GATES, ...HEAVY_GATES].map((g) => g.cmd[1]));
        const missing = docsJobScripts().filter((s) => !known.has(s) && !(s in NOT_RUN));
        expect(missing, "add each to GATES in scripts/lib/preflight-gates.mjs, or to NOT_RUN with the reason").toEqual([]);
    });

    it("lists no script that CI does not run, and no gate twice", () => {
        const ci = new Set(docsJobScripts());
        const own = GATES.map((g) => g.cmd[1]);
        expect(own.filter((s) => !ci.has(s))).toEqual([]);
        const ids = [...GATES, ...HEAVY_GATES].map((g) => g.id);
        expect(new Set(ids).size).toBe(ids.length);
    });

    it("names scripts that exist", () => {
        for (const g of GATES) expect(() => readFileSync(join(repo, g.cmd[1]))).not.toThrow();
    });
});

describe("preflight runner", () => {
    it("reads its options", () => {
        expect(parseArgs([]).jobs).toBe(4);
        expect(parseArgs(["--only", "a,b", "--jobs", "2", "--all", "--no-slot"])).toMatchObject({ only: ["a", "b"], jobs: 2, all: true, slot: false });
        expect(() => parseArgs(["--nope"])).toThrow(/unknown argument/);
    });

    it("says why a gate failed with the line that names the failure", () => {
        expect(reasonOf("checking\nFAIL docs/specs/X.md\n   Status must start with one of: draft proposed\n")).toBe("FAIL docs/specs/X.md");
        expect(reasonOf("one\ntwo\n")).toBe("two");
        expect(reasonOf("\x1b[31merror: bad thing\x1b[0m\n")).toBe("error: bad thing");
        expect(reasonOf("")).toBe("no output");
    });

    it("runs tsc and the type checks only when the branch changes what they read", () => {
        const tsc = HEAVY_GATES.find((g) => g.id === "tsc");
        expect(heavyWanted(tsc, ["crates/srv/src/a.rs"], false)).toBe(false);
        expect(heavyWanted(tsc, ["frontend/app/a.tsx"], false)).toBe(true);
        expect(heavyWanted(tsc, [], true)).toBe(true);
        expect(heavyWanted(tsc, ["test/contract/a.ts"], false)).toBe(true);
        // Both generators walk every .ts and .tsx file under frontend/.
        for (const id of ["rpc-stubs", "mux-events"]) {
            expect(heavyWanted(HEAVY_GATES.find((g) => g.id === id), ["frontend/app/foo.tsx"], false)).toBe(true);
        }
    });

    it("wants a changeset the branch adds, and says how to get one", () => {
        const files = {
            ".changesets/1-empty.md": "---\ntype: patch\n---\n",
            ".changesets/2-notype.md": "---\nbump: patch\n---\nA change.\n",
            ".changesets/3-badtype.md": "---\ntype: huge\n---\nA change.\n",
            ".changesets/4-nospace.md": "---\ntype:minor\n---\nA change.\n",
            ".changesets/5-crlf.md": "---\r\ntype: major\r\n---\r\nA change.\r\n",
        };
        const text = (p) => files[p] ?? "---\ntype: patch\n---\nA change.\n";
        const cs = ".changesets/1795279465-x-abcd.md";
        expect(changesetVerdict([["A", cs]], [], text)).toEqual({ ok: true });
        expect(changesetVerdict([["A", ".changesets/5-crlf.md"]], [], text)).toEqual({ ok: true });
        expect(changesetVerdict([["A", ".changesets/1-empty.md"]], [], text).reason).toMatch(/no description/);
        for (const bad of ["2-notype", "3-badtype", "4-nospace"]) {
            expect(changesetVerdict([["A", `.changesets/${bad}.md`]], [], text).reason).toMatch(/needs a first 'type: patch'/);
        }
        expect(changesetVerdict([["M", "a.rs"]], [], text).reason).toMatch(/scripts\/changeset\.sh/);
        expect(changesetVerdict([["M", "a.rs"]], [cs], text).reason).toBe(`${cs} is not added to git: run git add ${cs}`);
    });

    it("treats only a PR that updates the release files and removes a changeset as a release", () => {
        const cs = ".changesets/1795279465-x-abcd.md";
        const release = [["D", cs], ["M", "VERSION_HISTORY.md"], ["M", "package.json"]];
        expect(changesetVerdict(release, [], () => "").ok).toBe(true);
        expect(changesetVerdict([["D", cs]], [], () => "").ok).toBe(false);
        expect(changesetVerdict([["D", cs], ["M", "package.json"]], [], () => "").ok).toBe(false);
        expect(changesetVerdict([["M", "VERSION_HISTORY.md"], ["M", "package.json"]], [], () => "").ok).toBe(false);
    });

    it("refuses an --only id that names no gate", () => {
        expect(unknownIds(["tsc", "changeset", "cargo-check", "doc-status"])).toEqual([]);
        expect(unknownIds(["docs-statuss", "tsc"])).toEqual(["docs-statuss"]);
        expect(unknownIds(null)).toEqual([]);
    });
});
