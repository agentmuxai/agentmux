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
