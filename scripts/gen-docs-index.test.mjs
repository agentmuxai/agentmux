// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for scripts/gen-docs-index.mjs, the Node port of the specs-index
// generator (docs/specs/SPEC_DOCS_INDEX_GENERATOR_NODE_PORT_2026_09_23.md §7).
//
// The *.golden files under test-fixtures/gen-docs-index/ were produced by the
// ORIGINAL shell generator over the same fixture trees (make-goldens.mjs), so
// "matches the golden" means "byte-identical to the implementation it
// replaced". Cases where the port deliberately differs (spec §6) are asserted
// separately and say so.
//
// Runs on Linux, macOS and Windows in CI ("specs index (<os>)" in ci-pr.yml).

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
    BEGIN,
    asciiLower,
    byteCompare,
    bytes,
    lineDiff,
    main,
    spliceHead,
    statusOf,
    titleOf,
} from "./gen-docs-index.mjs";
import {
    GOLDEN_CASES,
    buildNoMarker,
    buildStatuses,
    commitAll,
    fixtureEnv,
    git,
    initRepo,
    makeDir,
    put,
    removeDir,
} from "./test-fixtures/gen-docs-index/fixtures.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = join(HERE, "gen-docs-index.mjs");
const WRAPPER = join(HERE, "gen-docs-index.sh");
const GOLDEN_DIR = join(HERE, "test-fixtures", "gen-docs-index");
const TIMEOUT = 60_000;

const dirs = [];
afterEach(() => {
    while (dirs.length) removeDir(dirs.pop());
});

function fixture(build) {
    const root = makeDir();
    dirs.push(root);
    if (build) build(root);
    return root;
}

/** Run the generator in-process against a fixture; capture everything. */
function run(root, args = [], extra = {}) {
    const outChunks = [];
    const errChunks = [];
    const code = main(args, {
        root,
        env: { ...fixtureEnv(root), ...(extra.env ?? {}) },
        out: (s) => outChunks.push(Buffer.isBuffer(s) ? s : Buffer.from(s, "utf8")),
        err: (s) => errChunks.push(Buffer.isBuffer(s) ? s : Buffer.from(s, "utf8")),
        transformRows: extra.transformRows,
    });
    return {
        code,
        out: Buffer.concat(outChunks).toString("utf8"),
        err: Buffer.concat(errChunks).toString("utf8"),
    };
}

const indexOf = (root) => readFileSync(join(root, "docs/specs/INDEX.md"));

// ── Pure functions ──────────────────────────────────────────────────────────

describe("statusOf (the shell pipeline's rules under LC_ALL=C)", () => {
    const cases = [
        ["**Status:** implemented — #1", "implemented"],
        ["**STATUS:** Proposed", "proposed"],
        ["**status:** draft", "draft"],
        ["**Status:** Draft, pending", "draft"],
        ["**Status:**\tliving", "living"],
        ["**Status:**   active", "active"],
        ["**Status:** active—Phase 0", "activephase"],
        ["**Status:** Alpha.", "alpha"],
        ["**Status:** — tbd", "__none__"],
        ["**Status:**", "__none__"],
        ["**Status:** proposed\r", "proposed"],
        ["  **Status:** implemented", "__none__"],
        ["Status: implemented", "__none__"],
    ];
    for (const [line, want] of cases) {
        it(`${JSON.stringify(line)} → ${want}`, () => {
            expect(statusOf(`# T\n\n${line}\n`)).toBe(want);
        });
    }

    it("treats U+00A0 as a word character, not a blank (awk's FS is space/tab/newline)", () => {
        expect(statusOf(bytes("**Status:** x\u00a0draft\n"))).toBe("xdraft");
    });

    it("uses the first Status line only", () => {
        expect(statusOf("**Status:** draft\n**Status:** implemented\n")).toBe("draft");
    });

    it("looks at the first 40 lines only", () => {
        const at = (n) => "x\n".repeat(n - 1) + "**Status:** active\n";
        expect(statusOf(at(40))).toBe("active");
        expect(statusOf(at(41))).toBe("__none__");
    });

    it("lowercases ASCII only", () => {
        expect(asciiLower(bytes("ÀBC"))).toBe(bytes("Àbc"));
    });
});

describe("titleOf", () => {
    it("takes the first '# ' line, strips '#' and spaces, deletes '|'", () => {
        expect(titleOf("# A | B || C\n", "f.md")).toBe("A  B  C");
        expect(titleOf("## Two\n# One\n", "f.md")).toBe("One");
        expect(titleOf("#NoSpace\n", "f.md")).toBe("f.md");
    });

    it("an empty first heading falls back to the filename, not a later heading", () => {
        expect(titleOf("#   \n# Later\n", "f.md")).toBe("f.md");
    });

    it("looks at the first 40 lines only", () => {
        expect(titleOf("x\n".repeat(40) + "# Late\n", "f.md")).toBe("f.md");
        expect(titleOf("x\n".repeat(39) + "# Line 40\n", "f.md")).toBe("Line 40");
    });

    it("drops a trailing CR (spec §6.2)", () => {
        expect(titleOf("# Title\r\n", "f.md")).toBe("Title");
    });

    it("keeps UTF-8 bytes unchanged", () => {
        expect(titleOf(bytes("# Café — ✓\n"), "f.md")).toBe(bytes("Café — ✓"));
    });
});

describe("byteCompare", () => {
    it("orders by byte value, uppercase before lowercase", () => {
        expect(["b", "a", "B", "_"].sort(byteCompare)).toEqual(["B", "_", "a", "b"]);
    });
});

describe("spliceHead", () => {
    it("keeps everything before the marker line", () => {
        const text = bytes(`curated\nmore\nprefix ${BEGIN}\nold\n`);
        expect(spliceHead(text)).toBe("curated\nmore\n");
    });

    it("appends a rule when there is no marker", () => {
        expect(spliceHead("curated")).toBe("curated\n---\n\n");
        expect(spliceHead("curated\n")).toBe("curated\n\n---\n\n");
    });
});

describe("lineDiff (diff(1) normal format)", () => {
    it("change, add, delete", () => {
        expect(lineDiff(["a", "b", "c"], ["a", "X", "c"])).toEqual(["2c2", "< b", "---", "> X"]);
        expect(lineDiff(["a", "c"], ["a", "b", "c"])).toEqual(["1a2", "> b"]);
        expect(lineDiff(["a", "b", "c"], ["a", "c"])).toEqual(["2d1", "< b"]);
        expect(lineDiff(["a"], ["a"])).toEqual([]);
    });

    // Property: the hunks, applied to `a`, reproduce `b` exactly.
    function apply(a, hunks) {
        const out = [];
        let ai = 0;
        let h = 0;
        while (h < hunks.length) {
            const m = /^(\d+)(?:,(\d+))?([acd])(\d+)(?:,(\d+))?$/.exec(hunks[h++]);
            expect(m, hunks[h - 1]).not.toBeNull();
            const [, s1, e1, op] = m;
            const aStart = Number(s1);
            const aEnd = e1 ? Number(e1) : aStart;
            const keepUntil = op === "a" ? aStart : aStart - 1;
            while (ai < keepUntil) out.push(a[ai++]);
            if (op !== "a") ai = aEnd;
            while (h < hunks.length && (hunks[h].startsWith("< ") || hunks[h] === "---")) h++;
            while (h < hunks.length && hunks[h].startsWith("> ")) out.push(hunks[h++].slice(2));
        }
        while (ai < a.length) out.push(a[ai++]);
        return out;
    }

    it("applying the hunks reproduces the new text (2,000 random cases)", () => {
        let seed = 12345;
        const rnd = (n) => {
            seed = (seed * 1103515245 + 12345) & 0x7fffffff;
            return seed % n;
        };
        const lines = (len) => Array.from({ length: len }, () => "abcde"[rnd(5)]);
        for (let i = 0; i < 2000; i++) {
            const a = lines(rnd(12));
            const b = lines(rnd(12));
            expect(apply(a, lineDiff(a, b)), JSON.stringify({ a, b })).toEqual(b);
        }
    });

    it("falls back to one change hunk for very large middles, still applicable", () => {
        const a = Array.from({ length: 2500 }, (_, i) => `a${i}`);
        const b = Array.from({ length: 2500 }, (_, i) => `b${i}`);
        const hunks = lineDiff(a, b);
        expect(hunks[0]).toBe("1,2500c1,2500");
        expect(apply(a, hunks)).toEqual(b);
    });

    it("multi-line ranges", () => {
        expect(lineDiff(["a", "b", "c", "d"], ["a", "X", "Y", "d"])).toEqual([
            "2,3c2,3",
            "< b",
            "< c",
            "---",
            "> X",
            "> Y",
        ]);
    });
});

// ── Golden trees: byte-identical to the shell generator ─────────────────────

describe("matches the shell generator byte-for-byte", () => {
    for (const [name, build] of Object.entries(GOLDEN_CASES)) {
        it(
            name,
            () => {
                const root = fixture(build);
                const res = run(root);
                expect(res.code, res.out + res.err).toBe(0);
                const golden = readFileSync(join(GOLDEN_DIR, `${name}.golden`));
                expect(indexOf(root).equals(golden)).toBe(true);
                // Idempotent: a second run is a no-op, and --check-all agrees.
                run(root);
                expect(indexOf(root).equals(golden)).toBe(true);
                expect(run(root, ["--check-all"]).code).toBe(0);
            },
            TIMEOUT
        );
    }

    it(
        "reports the spec count on stderr, excluding INDEX/README/archive/untracked/deleted",
        () => {
            const root = fixture(buildStatuses);
            const res = run(root);
            // 27 committed specs − deleted.md + staged.md = 27.
            expect(res.err).toContain("gen-docs-index: 27 specs under docs/specs/ (excluding archive/)");
            const text = indexOf(root).toString("utf8");
            expect(text).not.toContain("untracked.md");
            expect(text).not.toContain("deleted.md");
            expect(text).not.toContain("(old.md)");
            expect(text).not.toContain("(INDEX.md)");
            expect(text).not.toContain("(README.md)");
            expect(text).toContain("(staged.md)");
        },
        TIMEOUT
    );

    it(
        "a conflicted spec gets one row, not one per index stage",
        () => {
            const root = fixture(GOLDEN_CASES.conflict);
            run(root);
            const rows = indexOf(root)
                .toString("utf8")
                .split("\n")
                .filter((l) => l.startsWith("| [`c`]"));
            expect(rows).toHaveLength(1);
        },
        TIMEOUT
    );
});

// ── Deliberate differences from the shell generator (spec §6) ───────────────

describe("deliberate differences (spec §6)", () => {
    it(
        "§6.1 a non-ASCII filename gets a row (the shell version dropped it silently)",
        () => {
            const root = fixture((r) => {
                buildNoMarker(r);
                put(r, "docs/specs/Café.md", "# Accented name\n\n**Status:** draft\n");
                commitAll(r);
            });
            const res = run(root);
            expect(res.code).toBe(0);
            expect(res.err).toContain("3 specs under docs/specs/");
            expect(indexOf(root).toString("utf8")).toContain("| [`Café`](Café.md) | Accented name |\n");
        },
        TIMEOUT
    );

    it(
        "§6.2 a CRLF tree produces exactly what the same tree with LF produces",
        () => {
            const specs = {
                "one.md": "# One\n\n**Status:** implemented\n",
                "two.md": "# Two | pipe\n\n**Status:** Draft,\n",
                "three.md": "No title\n\n**Status:** zeta\n",
            };
            const make = (eol) => (r) => {
                initRepo(r);
                put(r, "docs/specs/INDEX.md", "# Specs index\n");
                for (const [n, t] of Object.entries(specs)) put(r, `docs/specs/${n}`, t.replace(/\n/g, eol));
                commitAll(r);
            };
            const lf = fixture(make("\n"));
            const crlf = fixture(make("\r\n"));
            run(lf);
            run(crlf);
            expect(indexOf(crlf).equals(indexOf(lf))).toBe(true);
            expect(indexOf(crlf).includes(Buffer.from("\r"))).toBe(false);
        },
        TIMEOUT
    );
});

// ── Modes, scope and failure handling ───────────────────────────────────────

/** main at base; a feature branch checked out on top. */
function branched(root, change) {
    buildNoMarker(root);
    run(root);
    commitAll(root, "index");
    git(root, "checkout", "-q", "-b", "feature");
    change(root);
    commitAll(root, "feature");
}

describe("--check", () => {
    it(
        "current → exit 0",
        () => {
            const root = fixture((r) =>
                branched(r, (x) => put(x, "docs/specs/one.md", "# One\n\n**Status:** implemented\n\nEdited body.\n"))
            );
            const res = run(root, ["--check"]);
            expect(res.code).toBe(0);
            expect(res.out).toContain("gen-docs-index: INDEX.md is current.");
        },
        TIMEOUT
    );

    it(
        "stale → exit 1 with a normal-format diff excerpt",
        () => {
            const root = fixture((r) =>
                branched(r, (x) => put(x, "docs/specs/two.md", "# Two\n\n**Status:** implemented\n"))
            );
            const before = indexOf(root);
            const res = run(root, ["--check"]);
            expect(res.code).toBe(1);
            expect(res.out).toContain("gen-docs-index: INDEX.md is STALE.");
            expect(res.out).toContain("Run: bash scripts/gen-docs-index.sh");
            // two.md moved from draft to implemented. Its row is identical on
            // both sides, so the minimal diff keeps it and deletes the draft
            // section around it — what diff(1) reports too.
            expect(res.out).toContain("    < ### draft");
            expect(res.out).toMatch(/\n {4}\d+(,\d+)?[acd]\d+(,\d+)?\n/);
            expect(indexOf(root).equals(before)).toBe(true);
        },
        TIMEOUT
    );

    it(
        "no spec or generator touched → not asserted, exit 0, even if stale",
        () => {
            const root = fixture((r) =>
                branched(r, (x) => {
                    put(x, "src/unrelated.txt", "x\n");
                })
            );
            // Make the committed index stale without touching docs/specs on the branch.
            put(root, "docs/specs/INDEX.md", "# Specs index\n\nhand edit\n");
            const res = run(root, ["--check"]);
            expect(res.code).toBe(0);
            expect(res.out).toContain("gen-docs-index: no specs changed on this branch — index not asserted.");
            expect(res.out).toContain("(run 'bash scripts/gen-docs-index.sh --check-all' to assert anyway)");
        },
        TIMEOUT
    );

    for (const gen of ["scripts/gen-docs-index.sh", "scripts/gen-docs-index.mjs"]) {
        it(
            `a change to ${gen} alone puts the index in scope`,
            () => {
                const root = fixture((r) => branched(r, (x) => put(x, gen, "// changed\n")));
                put(root, "docs/specs/INDEX.md", "# Specs index\n\nhand edit\n");
                commitAll(root, "stale index");
                expect(run(root, ["--check"]).code).toBe(1);
            },
            TIMEOUT
        );
    }

    it(
        "renaming a spec out of docs/specs puts the index in scope (both sides of the rename)",
        () => {
            const root = fixture((r) =>
                branched(r, (x) => {
                    git(x, "mv", "docs/specs/two.md", "notes.md");
                })
            );
            const res = run(root, ["--check"]);
            expect(res.code).toBe(1);
            expect(res.out).toContain("    < | [`two`](two.md) | Two |");
        },
        TIMEOUT
    );

    it(
        "an unresolvable base ref skips with exit 0",
        () => {
            const root = fixture((r) =>
                branched(r, (x) => put(x, "docs/specs/two.md", "# Two\n\n**Status:** implemented\n"))
            );
            const res = run(root, ["--check"], { env: { GITHUB_BASE_REF: "no-such-branch" } });
            expect(res.code).toBe(0);
            expect(res.out).toContain("gen-docs-index: cannot resolve base ref 'no-such-branch' — skipping.");
        },
        TIMEOUT
    );

    it(
        "prefers origin/<base> over <base>",
        () => {
            const root = fixture((r) =>
                branched(r, (x) => put(x, "docs/specs/two.md", "# Two\n\n**Status:** implemented\n"))
            );
            // origin/main points at the feature tip, so relative to it nothing changed.
            git(root, "update-ref", "refs/remotes/origin/main", "HEAD");
            expect(run(root, ["--check"]).out).toContain("no specs changed on this branch");
        },
        TIMEOUT
    );

    it(
        "--check-all ignores scope",
        () => {
            const root = fixture((r) => branched(r, (x) => put(x, "src/unrelated.txt", "x\n")));
            put(root, "docs/specs/INDEX.md", "# Specs index\n\nhand edit\n");
            expect(run(root, ["--check-all"]).code).toBe(1);
        },
        TIMEOUT
    );
});

describe("failure handling", () => {
    it(
        "missing INDEX.md → exit 1",
        () => {
            const root = fixture((r) => {
                initRepo(r);
                put(r, "docs/specs/one.md", "# One\n");
            });
            const res = run(root);
            expect(res.code).toBe(1);
            expect(res.out).toContain("gen-docs-index: docs/specs/INDEX.md not found");
        },
        TIMEOUT
    );

    it(
        "--check out of scope exits 0 before looking for INDEX.md (as before)",
        () => {
            const root = fixture((r) => branched(r, (x) => put(x, "src/unrelated.txt", "x\n")));
            rmSync(join(root, "docs/specs/INDEX.md"));
            expect(run(root, ["--check"]).code).toBe(0);
        },
        TIMEOUT
    );

    it(
        "completeness assertion: a lost row → exit 3 and INDEX.md untouched",
        () => {
            const root = fixture(buildNoMarker);
            const before = indexOf(root);
            const res = run(root, [], { transformRows: (m) => m.get("draft").pop() });
            expect(res.code).toBe(3);
            expect(res.err).toContain("gen-docs-index: FATAL — emitted 1 rows for 2 specs;");
            expect(res.err).toContain("refusing to write it.");
            expect(indexOf(root).equals(before)).toBe(true);
        },
        TIMEOUT
    );

    it(
        "leaves no temporary files behind",
        () => {
            const root = fixture(buildNoMarker);
            run(root);
            expect(readdirSync(join(root, "docs/specs")).sort()).toEqual(["INDEX.md", "one.md", "two.md"]);
        },
        TIMEOUT
    );

    it(
        "only the first argument is read; anything else regenerates",
        () => {
            const root = fixture(buildNoMarker);
            const res = run(root, ["--unknown", "--check"]);
            expect(res.code).toBe(0);
            expect(res.out).toContain("gen-docs-index: regenerated docs/specs/INDEX.md");
        },
        TIMEOUT
    );
});

// ── Entry points ────────────────────────────────────────────────────────────

describe("entry points", () => {
    it(
        "node scripts/gen-docs-index.mjs works as a CLI",
        () => {
            const root = fixture(buildNoMarker);
            const res = spawnSync(process.execPath, [SCRIPT, "--check-all"], { cwd: root, env: fixtureEnv(root) });
            expect(res.status).toBe(1);
            expect(res.stdout.toString()).toContain("INDEX.md is STALE.");
            const gen = spawnSync(process.execPath, [SCRIPT], { cwd: root, env: fixtureEnv(root) });
            expect(gen.status).toBe(0);
            expect(gen.stderr.toString()).toContain("2 specs under docs/specs/");
        },
        TIMEOUT
    );

    const hasBash = spawnSync("bash", ["-c", "exit 0"]).status === 0;
    it.skipIf(!hasBash)(
        "bash scripts/gen-docs-index.sh forwards arguments and the exit code",
        () => {
            const root = fixture(buildNoMarker);
            // Assert output, not just exit codes: a wrapper that fails to find
            // the .mjs also exits 1, which once passed the "stale" case by
            // coincidence (Windows paths carry backslashes).
            const stale = spawnSync("bash", [WRAPPER, "--check-all"], { cwd: root, env: fixtureEnv(root) });
            expect(stale.status, stale.stderr.toString()).toBe(1);
            expect(stale.stdout.toString()).toContain("INDEX.md is STALE.");
            const gen = spawnSync("bash", [WRAPPER], { cwd: root, env: fixtureEnv(root) });
            expect(gen.status, gen.stderr.toString()).toBe(0);
            expect(gen.stdout.toString()).toContain("regenerated docs/specs/INDEX.md");
            const ok = spawnSync("bash", [WRAPPER, "--check-all"], { cwd: root, env: fixtureEnv(root) });
            expect(ok.status).toBe(0);
            expect(ok.stdout.toString()).toContain("INDEX.md is current.");
        },
        TIMEOUT
    );

    it("the shell wrapper starts no processes besides node", () => {
        const src = readFileSync(WRAPPER, "utf8")
            .split("\n")
            .filter((l) => !l.trimStart().startsWith("#"))
            .join("\n");
        expect(src).not.toMatch(/\$\(|`/);
        expect(src).toMatch(/exec node /);
    });

    it("stays fast on a large tree", { timeout: TIMEOUT }, () => {
        const root = fixture((r) => {
            put(r, "docs/specs/INDEX.md", "# Specs index\n");
            for (let i = 0; i < 2000; i++) {
                put(
                    r,
                    `docs/specs/S${String(i).padStart(4, "0")}.md`,
                    `# Spec ${i}\n\n**Status:** draft\n\n${"body\n".repeat(50)}`
                );
            }
        });
        const t0 = performance.now();
        expect(run(root).code).toBe(0);
        // Generous bound for slow CI runners; the Windows dev machine does
        // 956 real specs in ~0.3 s. The shell version took minutes here.
        expect(performance.now() - t0).toBeLessThan(10_000);
    });
});

// Keep the golden directory honest: every golden has a case and vice versa.
it("every golden file has a fixture and every fixture has a golden", () => {
    const goldens = readdirSync(GOLDEN_DIR)
        .filter((f) => f.endsWith(".golden"))
        .map((f) => f.slice(0, -".golden".length))
        .sort();
    expect(goldens).toEqual(Object.keys(GOLDEN_CASES).sort());
    for (const g of goldens) expect(existsSync(join(GOLDEN_DIR, `${g}.golden`))).toBe(true);
});
