// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for lib/source-mask.mjs.

import { describe, expect, it } from "vitest";
import { blankCfgTest, cfgTestModules, maskRust, maskTs, rustCode } from "./lib/source-mask.mjs";

describe("maskRust", () => {
    it("keeps every offset and newline", () => {
        const src = 'let a = "x\ny"; // tail\nlet b = 1; /* a\nb */ let c = 2;\n';
        const got = maskRust(src);
        expect(got.length).toBe(src.length);
        expect(got.split("\n").length).toBe(src.split("\n").length);
        expect(got).toContain("let b = 1;");
        expect(got).toContain("let c = 2;");
        expect(got).not.toContain("tail");
    });

    it("blanks strings with escapes, raw strings and chars, and keeps braces in code", () => {
        const got = maskRust(`fn f() { let s = "a \\" { } "; let r = r##"x " { "##; let c = '{'; let d = '\\''; }`);
        expect(got.match(/\{/g)?.length).toBe(1);
        expect(got.match(/\}/g)?.length).toBe(1);
    });

    it("does not take a lifetime for a char literal", () => {
        const got = maskRust("fn f<'a>(x: &'a str) -> &'a str { x }");
        expect(got).toBe("fn f<'a>(x: &'a str) -> &'a str { x }");
    });

    it("skips a nested block comment as one", () => {
        expect(maskRust("a /* x /* y */ z */ b").replace(/\s+/g, " ")).toBe("a b");
    });

    it("leaves an identifier ending in r alone", () => {
        expect(maskRust('let for_r = r; let q = "x";')).toContain("let for_r = r;");
    });
});

describe("blankCfgTest", () => {
    it("blanks a test module and a test-only use, and nothing else", () => {
        const src = [
            "use a::b;",
            "#[cfg(test)]",
            "use a::only_tests;",
            "fn real() {}",
            "#[cfg(test)]",
            "mod tests {",
            "    fn t() { if x { y } }",
            "}",
            "fn tail() {}",
            "",
        ].join("\n");
        const got = rustCode(src);
        expect(got).toContain("use a::b;");
        expect(got).toContain("fn real() {}");
        expect(got).toContain("fn tail() {}");
        expect(got).not.toContain("only_tests");
        expect(got).not.toContain("fn t()");
        expect(got.split("\n").length).toBe(src.split("\n").length);
    });

    it("ends a cfg-gated field at its comma and leaves the next production item", () => {
        const src = [
            "struct Options {",
            "    pub a: u32,",
            "    #[cfg(test)]",
            "    pub b: HashMap<String, u32>,",
            "    #[cfg(test)]",
            "    #[allow(dead_code)]",
            "    pub c: u8,",
            "    pub d: u32,",
            "}",
            "const PROD: usize = crate::server::LIMIT;",
        ].join("\n");
        const got = rustCode(src);
        expect(got).toContain("pub a: u32,");
        expect(got).toContain("pub d: u32,");
        expect(got).toContain("const PROD: usize = crate::server::LIMIT;");
        expect(got).not.toContain("pub b");
        expect(got).not.toContain("pub c");
    });

    it("ends a cfg-gated function, const and match arm at their own end", () => {
        const src = [
            "#[cfg(test)]",
            "fn only_test() -> Vec<(u8, u8)> { vec![(1, 2)] }",
            "fn prod() {}",
            "#[cfg(test)]",
            "const T: u32 = 5;",
            "const P: u32 = 6;",
            "fn m(x: u8) { match x { #[cfg(test)] 1 => foo(), _ => bar() } }",
        ].join("\n");
        const got = rustCode(src);
        expect(got).not.toContain("only_test");
        expect(got).toContain("fn prod() {}");
        expect(got).not.toContain("const T");
        expect(got).toContain("const P: u32 = 6;");
        expect(got).not.toContain("foo()");
        expect(got).toContain("_ => bar()");
    });

    it("is not thrown by a < that compares, in a cfg-gated const or static", () => {
        const src = [
            "#[cfg(test)]",
            "const X: bool = 1 < 2;",
            "use crate::server::Prod;",
            "#[cfg(test)]",
            "static S: Foo = Foo { a: 1 < 2, b: 3 };",
            "fn p() { crate::server::a(); }",
        ].join("\n");
        const got = rustCode(src);
        expect(got).toContain("use crate::server::Prod;");
        expect(got).toContain("fn p() { crate::server::a(); }");
        expect(got).not.toContain("const X");
        expect(got).not.toContain("static S");
    });

    it("keeps the commas of generics inside a cfg-gated signature or field", () => {
        const src = [
            "#[cfg(test)]",
            "fn gated<A, B>(a: A, b: B) -> Result<A, B> { Ok(a) }",
            "fn prod() {}",
            "struct S {",
            "    #[cfg(test)]",
            "    f: HashMap<Vec<u8>, Option<String>>,",
            "    keep: u8,",
            "}",
        ].join("\n");
        const got = rustCode(src);
        expect(got).not.toContain("gated");
        expect(got).toContain("fn prod() {}");
        expect(got).not.toContain("HashMap");
        expect(got).toContain("keep: u8,");
    });

    it("never blanks past the first line of an item nothing ends", () => {
        const src = "#[cfg(test)]\nlet x = (1\nuse crate::server::Prod;\nfn p() {}\n";
        const got = rustCode(src);
        expect(got).toContain("fn p() {}");
    });

    it("is not thrown by a brace inside a string in the test module", () => {
        const src = ["#[cfg(test)]", "mod tests {", '    fn t() { let _ = "{"; }', "}", "fn after() {}"].join("\n");
        expect(rustCode(src)).toContain("fn after() {}");
        expect(blankCfgTest(maskRust(src))).toContain("fn after() {}");
    });
});

describe("cfgTestModules", () => {
    const at = (path, src) => cfgTestModules(path, maskRust(src));

    it("names the files of a module declared under cfg(test)", () => {
        const src = "mod real;\n#[cfg(test)]\nmod real_itest;\n#[cfg(test)]\n#[allow(dead_code)]\npub(crate) mod helpers;\n";
        expect(at("crates/srv/src/backend/mod.rs", src)).toEqual([
            "crates/srv/src/backend/real_itest.rs",
            "crates/srv/src/backend/real_itest/mod.rs",
            "crates/srv/src/backend/helpers.rs",
            "crates/srv/src/backend/helpers/mod.rs",
        ]);
    });

    it("looks in the folder named for a non-mod file, and ignores an inline module and a comment", () => {
        const src = "#[cfg(test)]\nmod tests;\n#[cfg(test)]\nmod inline { fn t() {} }\n// #[cfg(test)] mod no;\n";
        expect(at("crates/srv/src/backend/storage.rs", src)).toEqual([
            "crates/srv/src/backend/storage/tests.rs",
            "crates/srv/src/backend/storage/tests/mod.rs",
        ]);
    });
});

describe("maskTs offsets", () => {
    const SAMPLES = [
        'const quote = /["]/;\nconst lazy = () => import("@/app/view/v");\n',
        "<p>Don't stop</p>\nimport { a } from \"./a\";\n",
        'const s = "unterminated\nimport("x");\n',
        "const t = `a\nb ${c}\n`; const r = /[/]x/g;\n",
        "const half = a / b; const other = c / d;\nimport { z } from \"./z\";\n",
        "if (x) return /'/.test(y);\nimport(\"./k\");\n",
        "const e = 'it\\\\'s'; // note\n/* a\nb */ import \"./side\";\n",
    ];

    it("keeps the length and every newline for each sample", () => {
        for (const src of SAMPLES) {
            const got = maskTs(src);
            expect(got.length, src).toBe(src.length);
            expect([...got].map((ch, k) => (ch === "\n" ? k : -1)).filter((k) => k >= 0)).toEqual(
                [...src].map((ch, k) => (ch === "\n" ? k : -1)).filter((k) => k >= 0)
            );
        }
    });

    it("does not take a quote in a regex literal for a string, so a later import is still found", () => {
        const got = maskTs(SAMPLES[0]);
        expect(got).toContain('import("');
        expect(got).not.toContain("/[\"]/");
    });

    it("tells a division from a regex literal", () => {
        const got = maskTs(SAMPLES[4]);
        expect(got).toContain("a / b");
        expect(got).toContain("c / d");
        expect(got).toContain("import { z }");
    });

    it("does not take a quote nothing closes on its line for a string, so the next line is read", () => {
        const got = maskTs(SAMPLES[2]);
        expect(got.split("\n")[1]).toBe('import(" ");');
    });

    it("keeps the bracket after an apostrophe in JSX text", () => {
        const got = maskTs("const a = <Show fallback={<p>Open AgentMux's app.</p>}>x</Show>;\n");
        expect(got.match(/\{/g)?.length).toBe(1);
        expect(got.match(/\}/g)?.length).toBe(1);
    });

    it("takes the / of a closing JSX tag for a tag, and the / after an arrow for a pattern", () => {
        const jsx = maskTs("const a = <For each={x}>{(k) => <K>{k}</K>}</For>;\n");
        expect(jsx.match(/\{/g)?.length).toBe(3);
        expect(jsx.match(/\}/g)?.length).toBe(3);
        expect(maskTs("const t = (s) => s.filter((x) => /[(]/.test(x));\n")).not.toContain("[(]");
    });
});

describe("maskTs with JSX", () => {
    const jsx = (src) => maskTs(src, { jsx: true });

    it("blanks the text between tags, keeps the expressions in it, and the length", () => {
        const src = "const a = <div>Open (it's) {f(x)} <b>{y}</b> end</div>;\nconst z = 1;\n";
        const got = jsx(src);
        expect(got.length).toBe(src.length);
        expect(got).toContain("{f(x)}");
        expect(got).toContain("<b>{y}</b>");
        expect(got).not.toContain("Open");
        expect(got).toContain("const z = 1;");
    });

    it("reads attributes, self-closing tags, fragments and nesting in expressions", () => {
        const src = [
            'const a = (<><A title="a > b" n={() => { go(); }} /><B>{ok ? <i>(</i> : null}</B></>);',
            "const z = 1;",
            "",
        ].join("\n");
        const got = jsx(src);
        expect(got).toContain("const z = 1;");
        expect(got).toContain("go();");
        expect(got).not.toContain("a > b");
        expect(got.match(/\(/g)?.length).toBe(got.match(/\)/g)?.length);
    });

    it("reads type arguments on a tag, a comment between attributes, and a closing fragment", () => {
        const src = [
            "const a = (",
            "    <>",
            "        <List<Row<P>>",
            "            // it's (not) an attribute",
            "            title='x'",
            "            onGo={() => { go(); }}",
            "        >",
            "            row (",
            "        </List>",
            "    </>",
            ");",
            "const z = (1);",
            "",
        ].join("\n");
        const got = jsx(src);
        expect(got.length).toBe(src.length);
        expect(got).toContain("go();");
        expect(got).not.toContain("row");
        expect(got).not.toContain("not");
        expect(got).toContain("const z = (1);");
        expect(got.match(/\(/g)?.length).toBe(got.match(/\)/g)?.length);
    });

    it("keeps lines, and does not take a comparison or a generic arrow for an element", () => {
        const src = "const f = <T,>(x: T) => x;\nconst g = a <b && c> d;\nconst h = (x: number) => (x < 3) ? 1 : 2;\n";
        expect(jsx(src)).toBe(src);
    });

    it("leaves `<A>x` alone without the option, as a type assertion in a .ts file", () => {
        const src = "const a = <Foo>bar;\nconst b = (1);\n";
        expect(maskTs(src)).toBe(src);
    });
});

describe("maskTs", () => {
    it("blanks comments and the inside of strings and templates, keeping offsets", () => {
        const src = 'import a from "./a"; // note\nconst t = `x ${y} /* z */`; /* b */ const u = \'it\\\'s\';\n';
        const got = maskTs(src);
        expect(got.length).toBe(src.length);
        expect(got.split("\n").length).toBe(src.split("\n").length);
        expect(got).toContain("import a from");
        expect(got).not.toContain("note");
        expect(got).not.toContain("${y}");
        expect(got).toContain("const u =");
        expect(got).toContain('"   "');
    });

    it("does not take // inside a string for a comment", () => {
        expect(maskTs('const u = "http://x"; const v = 1;')).toContain("const v = 1;");
    });
});
