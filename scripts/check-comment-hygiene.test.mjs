// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-comment-hygiene.mjs's pure parts: the lexer, the
// narration rules, the diff parser, the gate and the code-equal comparison.
// No git and no filesystem. The narration examples are real comments from
// `main` (see docs/reports/REPORT_COMMENT_COMPRESSION_WORTH_IT_2026_10_02.md).

import { describe, expect, it } from "vitest";
import {
    buildRefIndex,
    commentRefs,
    compareCodeEqual,
    deadRefFindings,
    gateFile,
    isForeignRef,
    isScanPath,
    isSourcePath,
    lexSource,
    narrationRule,
    normalizeComment,
    parseAddedLines,
    parseDiff,
    protectedCounts,
    refKind,
    resolvesRef,
    staleAfterMove,
    statsOf,
} from "./check-comment-hygiene.mjs";

const ts = (src) => lexSource(src, "ts");
const rs = (src) => lexSource(src, "rs");
const textOf = (info) => info.lines.map((l) => l.text);

describe("lexSource: TypeScript", () => {
    it("separates comment-only lines, code lines and trailing comments", () => {
        const info = ts("// a\nconst x = 1; // b\n\nfoo();\n");
        expect(info.lines.map((l) => [l.code, l.text])).toEqual([
            [false, "// a"],
            [true, "// b"],
            [false, ""],
            [true, ""],
            [false, ""],
        ]);
    });

    it("does not read // inside strings or template literals as a comment", () => {
        const info = ts("const u = 'http://x.test'; const v = \"//y\"; const w = `//${a}//z`;\n");
        expect(textOf(info).join("")).toBe("");
        expect(info.lines[0].code).toBe(true);
    });

    it("handles a multi-line block comment and a JSX comment", () => {
        const info = ts("/* one\n   two */\nconst a = (\n  <div>{/* jsx note */}</div>\n);\n");
        expect(info.lines[0].text).toBe("/* one");
        expect(info.lines[1].text.trim()).toBe("two */");
        expect(info.lines[1].code).toBe(false);
        expect(info.lines[3].text).toBe("/* jsx note */");
        expect(info.lines[3].code).toBe(true);
    });

    it("does not read a regex literal containing // as a comment", () => {
        const info = ts("const r = /https?:\\/\\//; // real\n");
        expect(info.lines[0].text).toBe("// real");
    });

    it("reads a regex literal after an arrow, keeping its whitespace and hiding its //", () => {
        expect(ts("const r = () => /a b/;").code).not.toBe(ts("const r = () => /a  b/;").code);
        expect(ts("const r = () => /x\\/\\//; // real").lines[0].text).toBe("// real");
        // A comparison is still not a regex start.
        expect(ts("const c = a > /* note */ b;").code).toBe(ts("const c = a > b;").code);
    });

    it("does not read JSX closing tags as a regex", () => {
        const info = ts("const a = <p>x</p>; // note\n");
        expect(info.lines[0].text).toBe("// note");
    });

    it("stops an unterminated apostrophe at the end of the line", () => {
        const info = ts("const a = <p>don't</p>;\n// next\n");
        expect(info.lines[1].text).toBe("// next");
        expect(info.lines[1].code).toBe(false);
    });

    it("stops a double-quoted JSX text at the line end too", () => {
        const info = ts('const a = <p>Choose 6"</p>;\n// next\n');
        expect(info.lines[1].text).toBe("// next");
        expect(info.lines[1].code).toBe(false);
    });

    it("finds a comment inside a template substitution, and resumes the string after it", () => {
        const info = ts("const s = `a ${foo( // inside\n  x)} // text`;\n// after\n");
        expect(info.lines[0].text).toBe("// inside");
        expect(info.lines[1].text).toBe("");
        expect(info.lines[2].text).toBe("// after");
        expect(info.lines[2].code).toBe(false);
    });

    it("counts braces and nested templates inside a substitution", () => {
        const info = ts("const s = `${ {a: 1}.a } ${ `n ${y} // str` /* c */ } // str2`;\n");
        expect(info.lines[0].text).toBe("/* c */");
        expect(ts("const s = `${ {a: 1}.a }`; // t\n").lines[0].text).toBe("// t");
    });

    it("does not treat a multi-line template literal's lines as comments", () => {
        const info = ts("const q = `\n// not a comment\n`;\n");
        expect(textOf(info).join("")).toBe("");
    });
});

describe("lexSource: Rust", () => {
    it("tags doc comments", () => {
        const info = rs("/// doc\n//! inner\n// line\nfn f() {}\n");
        expect(info.lines[0].kinds.has("doc")).toBe(true);
        expect(info.lines[1].kinds.has("doc")).toBe(true);
        expect(info.lines[2].kinds.has("doc")).toBe(false);
        expect(info.lines[2].kinds.has("line")).toBe(true);
    });

    it("does not read // inside a raw string", () => {
        const info = rs('let s = r#"a // b "# ; // c\n');
        expect(info.lines[0].text).toBe("// c");
    });

    it("tells lifetimes from char literals", () => {
        const info = rs("fn f<'a>(x: &'a str) -> char { '\\'' } // after\n");
        expect(info.lines[0].text).toBe("// after");
        const chars = rs("let c = 'a'; let d = '\"'; // tail\n");
        expect(chars.lines[0].text).toBe("// tail");
    });

    it("nests block comments", () => {
        const info = rs("/* a /* b */ still */ let x = 1;\n");
        expect(info.lines[0].text).toBe("/* a /* b */ still */");
        expect(info.lines[0].code).toBe(true);
        expect(info.code).toBe("let x = 1;");
    });
});

describe("lexSource: code stream", () => {
    it("ignores comments and whitespace outside strings", () => {
        const a = ts("const x   =  1; // note\nfoo( 1 );\n");
        const b = ts("// moved\nconst x = 1;\n/* more */ foo( 1 );\n");
        expect(a.code).toBe(b.code);
    });

    it("keeps a line terminator significant (automatic semicolon insertion)", () => {
        const asi = ts("function f() {\n  return\n  // why\n  value;\n}\n");
        expect(asi.code).not.toBe(ts("function f() {\n  return value;\n}\n").code);
        // Deleting only the comment line keeps the terminator, so the code is the same.
        expect(asi.code).toBe(ts("function f() {\n  return\n  value;\n}\n").code);
        // A block comment that spans lines is itself a line terminator.
        expect(ts("return /* a\n b */ x;").code).not.toBe(ts("return /* a b */ x;").code);
    });

    it("ignores blank lines and a removed trailing comment", () => {
        expect(ts("a;\n\n\nb;\n").code).toBe(ts("a;\nb;\n").code);
        expect(ts("a; // note\nb;\n").code).toBe(ts("a;\nb;\n").code);
    });

    it("keeps a comment between tokens from gluing them together", () => {
        expect(ts("a/* x */b").code).toBe("a b");
    });

    it("preserves whitespace inside string literals", () => {
        expect(ts("const s = 'a  b';").code).not.toBe(ts("const s = 'a b';").code);
    });
});

describe("narrationRule", () => {
    it.each([
        ["reagentx P1 on PR #2338 (thirty-fifth re-review)."],
        ["// codex P1 on PR #2371: the JoinHandle is kept"],
        ["Final bound (Phase 3 spec round 8 / round 9)"],
        ["(Codex, PR #3152). Past this budget"],
        ["the exact attack this fix closes (reagent + Codex, PR #2662, 2026-08-19)"],
        ["reagent caught this on the first draft"],
        ["the actual safety property Codex asked for"],
        ["PR #1234 review (P1): keep the guard"],
        ["fixed in #2338, flagged as P2 at the time"],
    ])("flags %s", (text) => {
        expect(narrationRule(text)).not.toBeNull();
    });

    it.each([
        ["ReAgent review notifications arrive as jekts"],
        ["signed by the reagent WAN key (reagent-v1-dev)"],
        ["lerp from p0 to p1, then clamp to #333"],
        ["rounds the value to the nearest pixel"],
        ["a P1 bug here would block login"],
        ["see PR #2338 for the design"],
        ["Rationale: the constraint holds (#2338)."],
    ])("leaves %s alone", (text) => {
        expect(narrationRule(text)).toBeNull();
    });

    it("honours the opt-out token", () => {
        expect(narrationRule("round 2 of the handshake // comment-hygiene: allow")).toBeNull();
    });
});

describe("parseAddedLines", () => {
    it("reads the new-side line ranges of each hunk", () => {
        const diff = [
            "diff --git a/x.rs b/x.rs",
            "--- a/x.rs",
            "+++ b/x.rs",
            "@@ -3,0 +4,2 @@ fn a()",
            "+one",
            "+two",
            "@@ -10 +12 @@",
            "-old",
            "+new",
            "@@ -20,2 +21,0 @@",
            "diff --git a/gone.ts b/gone.ts",
            "--- a/gone.ts",
            "+++ /dev/null",
            "@@ -1,3 +0,0 @@",
        ].join("\n");
        const added = parseAddedLines(diff);
        expect([...added.get("x.rs")]).toEqual([4, 5, 12]);
        expect(added.has("gone.ts")).toBe(false);
    });
});

describe("parseDiff", () => {
    it("also reads the old-side ranges, keyed by old path", () => {
        const diff = [
            "diff --git a/old.rs b/new.rs",
            "--- a/old.rs",
            "+++ b/new.rs",
            "@@ -5,3 +7,0 @@",
            "-x",
            "-y",
            "-z",
            "@@ -20 +21 @@",
            "-q",
            "+r",
        ].join("\n");
        const { added, removed } = parseDiff(diff);
        expect([...removed.get("old.rs")]).toEqual([5, 6, 7, 20]);
        expect([...added.get("new.rs")]).toEqual([21]);
    });

    it("does not read a changed line that starts with -- or ++ as a file header", () => {
        const diff = ["diff --git a/x.rs b/x.rs", "--- a/x.rs", "+++ b/x.rs", "@@ -1 +1 @@", "--- a/not-a-header", "+++ b/not-a-header"].join("\n");
        const { added, removed } = parseDiff(diff);
        expect([...added.keys()]).toEqual(["x.rs"]);
        expect([...removed.keys()]).toEqual(["x.rs"]);
    });
});

describe("normalizeComment", () => {
    it("drops markers and spacing so a moved comment compares equal", () => {
        const n = normalizeComment;
        expect(n("    // reagent P1 on PR #2338:  keep")).toBe("reagent P1 on PR #2338: keep");
        expect(n("/// reagent P1 on PR #2338: keep")).toBe("reagent P1 on PR #2338: keep");
        expect(n("   * reagent P1 on PR #2338: keep")).toBe("reagent P1 on PR #2338: keep");
        expect(n("/* reagent P1 on PR #2338: keep */")).toBe("reagent P1 on PR #2338: keep");
    });
});

describe("gateFile: moved comments", () => {
    it("does not hold a moved comment to the narration rule, but still catches a new one", () => {
        const info = rs("// reagent P1 on PR #2338: moved\nfn a() {}\n// codex P2 on PR #4000: brand new\nfn b() {}\n");
        const moved = new Set([normalizeComment("// reagent P1 on PR #2338: moved")]);
        expect(gateFile(info, new Set([1, 3]), moved).errors.map((e) => e.line)).toEqual([3]);
        expect(gateFile(info, new Set([1, 3])).errors.map((e) => e.line)).toEqual([1, 3]);
    });
});

describe("gateFile", () => {
    const body = ["// reagent P1 on PR #2338 (re-review): keep", "fn a() {}", "// plain why", "fn b() {}"].join("\n");

    it("fails narration on added lines only", () => {
        const info = rs(body);
        expect(gateFile(info, new Set([1])).errors.map((e) => e.line)).toEqual([1]);
        expect(gateFile(info, new Set([3])).errors).toEqual([]);
    });

    it("does not look at code or string content", () => {
        const info = rs('let s = "reagent P1 on PR #2338";\n');
        expect(gateFile(info, new Set([1])).errors).toEqual([]);
    });

    it("warns, without failing, on a new long // block but not on a long doc block", () => {
        const long = (p) => Array.from({ length: 10 }, (_, i) => `${p} line ${i}`).join("\n") + "\nfn f() {}\n";
        const lines = new Set(Array.from({ length: 10 }, (_, i) => i + 1));
        const plain = gateFile(rs(long("//")), lines);
        expect(plain.errors).toEqual([]);
        expect(plain.warnings).toHaveLength(1);
        expect(gateFile(rs(long("///")), lines).warnings).toEqual([]);
    });
});

describe("compareCodeEqual", () => {
    const before = "// SAFETY: sound because x (#2338)\nunsafe { f(); }\n// long narration, SPEC_FOO_2026_01_01.md\nlet s = \"a  b\";\n";

    it("accepts a comment-only edit and lists the citations it dropped", () => {
        const after = "// SAFETY: sound because x\nunsafe { f(); }\nlet s = \"a  b\";\n";
        const res = compareCodeEqual(before, after, "rs");
        expect(res.problems).toEqual([]);
        expect(res.dropped).toEqual(["#2338", "SPEC_FOO_2026_01_01"]);
    });

    it("rejects any code change, including inside a string", () => {
        const edited = before.replace('"a  b"', '"a b"');
        expect(compareCodeEqual(before, edited, "rs").problems[0]).toMatch(/code changed/);
        expect(compareCodeEqual(before, before.replace("f()", "g()"), "rs").problems[0]).toMatch(/code changed/);
    });

    it("rejects removing a SAFETY comment or a TODO", () => {
        const noSafety = before.replace("// SAFETY: sound because x (#2338)\n", "");
        expect(compareCodeEqual(before, noSafety, "rs").problems.join("|")).toMatch(/SAFETY:/);
        const withTodo = "// TODO: later\nlet a = 1;\n";
        expect(compareCodeEqual(withTodo, "let a = 1;\n", "rs").problems.join("|")).toMatch(/TODO/);
    });
});

describe("statsOf / protectedCounts / isSourcePath", () => {
    it("counts comment-only lines and ratio", () => {
        const s = statsOf(rs("// a\n// b\nfn f() {} // c\nfn g() {}\n"));
        expect(s.commentOnly).toBe(2);
        expect(s.lines).toBe(5);
    });

    it("counts directives only in comment text", () => {
        const c = protectedCounts(ts("// eslint-disable-next-line x\nconst s = 'TODO';\n// @ts-expect-error y\n"));
        expect(c.TODO).toBe(0);
        expect(c["eslint directive"]).toBe(1);
        expect(c["@ts directive"]).toBe(1);
    });

    it("protects bundler, test-runner and formatter directives", () => {
        const before = "const m = import(/* @vite-ignore */ url);\n/** @vitest-environment jsdom */\n// prettier-ignore\nconst t = 1;\n";
        const lost = "const m = import(/* dynamic */ url);\n/** jsdom */\n// keep\nconst t = 1;\n";
        const problems = compareCodeEqual(before, lost, "ts").problems.join("|");
        expect(problems).toMatch(/bundler hint/);
        expect(problems).toMatch(/test-runner directive/);
        expect(problems).toMatch(/formatter or linter switch/);
        expect(compareCodeEqual(before, before, "ts").problems).toEqual([]);
    });

    it("skips generated and vendored paths", () => {
        expect(isSourcePath("crates/srv/src/a.rs")).toBe(true);
        expect(isSourcePath("frontend/app/a.tsx")).toBe(true);
        expect(isSourcePath("frontend/types/rpc/Foo.ts")).toBe(false);
        expect(isSourcePath("node_modules/x/a.ts")).toBe(false);
        expect(isSourcePath("docs/a.md")).toBe(false);
    });
});

describe("file references", () => {
    const index = buildRefIndex([
        "crates/srv/src/backend/blockcontroller/persistent/spawn.rs",
        "crates/srv/src/backend/blockcontroller/persistent/mod.rs",
        "frontend/app/view/agent/agent-view.tsx",
        "docs/specs/SPEC_FOO_2026_01_01.md",
    ]);

    it("finds names in comments only, and joins a name wrapped across lines", () => {
        const info = rs(
            [
                '// See persistent/spawn.rs and SPEC_FOO_',
                "// 2026_01_01.md for why.",
                'let s = "not/a/comment.rs";',
            ].join("\n"),
        );
        expect(commentRefs(info).map((r) => [r.line, r.ref])).toEqual([
            [1, "persistent/spawn.rs"],
            [2, "SPEC_FOO_2026_01_01.md"],
        ]);
    });

    it("does not join an SCSS partial or a `--` separator onto the previous line", () => {
        const partial = ts("// sibling of\n// _cpu-cores-popover.scss (same chrome)\n");
        expect(commentRefs(partial).map((r) => r.ref)).toEqual(["_cpu-cores-popover.scss"]);
        const dashes = ts("/* stylelint-disable color-no-hex --\n   theme.scss is the canonical source */\n");
        expect(commentRefs(dashes).map((r) => r.ref)).toEqual(["theme.scss"]);
    });

    it("reads `a.ts/b.ts` as two names, not a path", () => {
        const refs = commentRefs(ts("// see ToolBlock.tsx/MarkdownBlock.tsx and src/app/x.ts\n")).map((r) => r.ref);
        expect(refs).toEqual(["ToolBlock.tsx", "MarkdownBlock.tsx", "src/app/x.ts"]);
    });

    it("resolves by full path, path suffix, relative path or basename", () => {
        expect(resolvesRef("frontend/app/view/agent/agent-view.tsx", index)).toBe(true);
        expect(resolvesRef("persistent/spawn.rs", index)).toBe(true);
        expect(resolvesRef("../agent/agent-view.tsx", index)).toBe(true);
        expect(resolvesRef("spawn.rs", index)).toBe(true);
        expect(resolvesRef("persistent.rs", index)).toBe(false);
        expect(resolvesRef("other/spawn.rs", index)).toBe(false);
    });

    it("tells doc names, repo-rooted paths and bare names apart", () => {
        expect(refKind("SPEC_BAR_2026-05-14.md", index)).toBe("doc");
        expect(refKind("docs/specs/x.md", index)).toBe("doc");
        expect(refKind("crates/srv/src/app.rs", index)).toBe("path");
        expect(refKind("identity/resolver.rs", index)).toBe("bare");
        expect(refKind("persistent.rs", index)).toBe("bare");
    });

    it("treats runtime files, other repos and elided paths as foreign", () => {
        expect(isForeignRef("CLAUDE.md")).toBe(true);
        expect(isForeignRef(".claude/CLAUDE.md")).toBe(true);
        expect(isForeignRef("muxbus/server/src/wan-keys.ts")).toBe(true);
        expect(isForeignRef("frontend/.../runtime-apply.ts")).toBe(true);
        expect(isForeignRef("src-tauri/src/commands/drag.rs")).toBe(true);
        expect(isForeignRef(".github/copilot-instructions.md")).toBe(true);
        expect(isForeignRef("agentmux-cloud/docs/PLAN_X.md")).toBe(true);
        expect(isForeignRef("persistent.rs")).toBe(false);
        expect(isForeignRef("crates/srv/src/app.rs")).toBe(false);
    });

    it("errors on an added dead doc or repo path, warns on a bare name, skips the rest", () => {
        const info = rs(
            [
                "// See SPEC_BAR_2026-05-14.md.",
                "// Lives in crates/srv/src/gone.rs.",
                "// Was in persistent.rs.",
                "// Writes CLAUDE.md into the workdir.",
                "// See persistent/spawn.rs.",
                "// Old: crates/srv/src/old.rs // comment-hygiene: allow",
                "// Not added: SPEC_NOPE.md",
            ].join("\n"),
        );
        const res = deadRefFindings(info, new Set([1, 2, 3, 4, 5, 6]), index);
        expect(res.errors.map((e) => e.line)).toEqual([1, 2]);
        expect(res.warnings.map((w) => w.line)).toEqual([3]);
        const moved = new Set(["See SPEC_BAR_2026-05-14.md."]);
        expect(deadRefFindings(info, new Set([1]), index, moved).errors).toEqual([]);
    });

    it("flags comments naming a file the branch renamed or deleted", () => {
        const info = rs(["// see persistent.rs", "// and blockcontroller/persistent.rs", "// but other/persistent.rs is unrelated", "// history: split from persistent.rs // comment-hygiene: allow"].join("\n"));
        const gone = [{ path: "crates/srv/src/backend/blockcontroller/persistent.rs", to: "crates/srv/src/backend/blockcontroller/persistent/mod.rs" }];
        const out = staleAfterMove(gone, [{ file: "a.rs", info }]);
        expect(out.map((o) => o.line)).toEqual([1, 2]);
        expect(out[0].message).toMatch(/renames `crates\/srv\/src\/backend\/blockcontroller\/persistent.rs` to/);
        expect(staleAfterMove([{ path: "x/gone.ts", to: null }], [{ file: "b.ts", info: ts("// uses gone.ts") }])[0].message).toMatch(/deletes `x\/gone.ts`/);
    });
    it("matches every removed file that shares a basename", () => {
        const gone = [
            { path: "crates/a/src/util.rs", to: "crates/a/src/util/mod.rs" },
            { path: "crates/b/src/util.rs", to: null },
        ];
        const info = rs(["// see a/src/util.rs", "// see b/src/util.rs", "// see util.rs"].join("\n"));
        const out = staleAfterMove(gone, [{ file: "c.rs", info }]);
        expect(out.map((o) => o.line)).toEqual([1, 2, 3]);
        expect(out[1].message).toMatch(/deletes `crates\/b\/src\/util.rs`/);
        expect(out[2].message).toMatch(/removes every file of that name/);
    });

    it("checks .js paths but treats bare .js names as libraries, except when the file is removed", () => {
        expect(isForeignRef("xterm.js")).toBe(true);
        expect(isForeignRef("Node.js")).toBe(true);
        expect(isForeignRef("crates/cef/src/browser_api/scripts/gone.js")).toBe(false);
        expect(commentRefs(ts("// see scripts/query.js\n")).map((r) => r.ref)).toEqual(["scripts/query.js"]);
        const gone = [{ path: "crates/cef/src/browser_api/scripts/query.js", to: null }];
        expect(staleAfterMove(gone, [{ file: "a.rs", info: rs("// injects query.js") }]).map((o) => o.line)).toEqual([1]);
    });

    it("scans JavaScript-family files in tree-wide checks", () => {
        expect(isScanPath("scripts/check-doc-links.mjs")).toBe(true);
        expect(isScanPath("crates/srv/src/backend/shellintegration/muxlog.mjs")).toBe(true);
        expect(isScanPath("version.cjs")).toBe(true);
        expect(isScanPath("node_modules/x/index.js")).toBe(false);
        expect(isScanPath("docs/a.md")).toBe(false);
    });

    it("recognises a sibling repo behind ./ or ../", () => {
        expect(isForeignRef("../agentmux-cloud/docs/SPEC_X.md")).toBe(true);
        expect(isForeignRef("./agentmux-cloud/docs/SPEC_X.md")).toBe(true);
    });
});
