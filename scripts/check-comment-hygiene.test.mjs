// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-comment-hygiene.mjs's pure parts: the lexer, the
// narration rules, the diff parser, the gate and the code-equal comparison.
// No git and no filesystem. The narration examples are real comments from
// `main` (see docs/reports/REPORT_COMMENT_COMPRESSION_WORTH_IT_2026_10_02.md).

import { describe, expect, it } from "vitest";
import {
    compareCodeEqual,
    gateFile,
    isSourcePath,
    lexSource,
    narrationRule,
    parseAddedLines,
    protectedCounts,
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

    it("does not read JSX closing tags as a regex", () => {
        const info = ts("const a = <p>x</p>; // note\n");
        expect(info.lines[0].text).toBe("// note");
    });

    it("stops an unterminated apostrophe at the end of the line", () => {
        const info = ts("const a = <p>don't</p>;\n// next\n");
        expect(info.lines[1].text).toBe("// next");
        expect(info.lines[1].code).toBe(false);
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

    it("skips generated and vendored paths", () => {
        expect(isSourcePath("crates/srv/src/a.rs")).toBe(true);
        expect(isSourcePath("frontend/app/a.tsx")).toBe(true);
        expect(isSourcePath("frontend/types/rpc/Foo.ts")).toBe(false);
        expect(isSourcePath("node_modules/x/a.ts")).toBe(false);
        expect(isSourcePath("docs/a.md")).toBe(false);
    });
});
