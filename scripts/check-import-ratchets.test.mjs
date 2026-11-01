// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-import-ratchets.mjs.

import { describe, expect, it } from "vitest";
import {
    aliasesOf,
    compareCounts,
    compareSets,
    filesInCycles,
    importGraph,
    invertedImports,
    isTypeOnlyClause,
    loweredCounts,
    loweredSet,
    parseCounts,
    parseLines,
    readImports,
    resolveSpecifier,
    serverRefs,
} from "./check-import-ratchets.mjs";

const ALIASES = aliasesOf(
    JSON.stringify({
        compilerOptions: {
            paths: { "@/app/*": ["frontend/app/*"], "@/util/*": ["frontend/util/*"], "@host-module": ["frontend/cef-host-module.ts"] },
        },
    })
);

describe("serverRefs", () => {
    it("counts every crate::server path in production code", () => {
        const src = `use crate::server::AppState;
fn f() { crate::server::agent_runtime::list(); }
`;
        expect(serverRefs(src)).toBe(2);
    });

    it("skips comments, cfg(test) modules and cfg(test) items", () => {
        const src = `// use crate::server::Nope;
use crate::server::Yes;
#[cfg(test)]
mod tests {
    use crate::server::tests::test_state;
    fn t() { let _ = crate::server::x(); }
}
#[cfg(test)]
use crate::server::OnlyForTests;
fn tail() {}
`;
        expect(serverRefs(src)).toBe(1);
    });

    it("does not count text in a string, a raw string or a block comment", () => {
        const src = `fn f() {
    log("see crate::server::thing");
    let _ = r#"crate::server::raw"#;
    /* crate::server::block */
    let _ = crate::server::real();
}
`;
        expect(serverRefs(src)).toBe(1);
    });

    it("does not take a longer name for the server module", () => {
        expect(serverRefs("use crate::server_state::X; use crate::servers::Y;")).toBe(0);
    });
});

describe("isTypeOnlyClause", () => {
    it("reads import type, export type and an all-type named list as type only", () => {
        expect(isTypeOnlyClause(" type { A } ")).toBe(true);
        expect(isTypeOnlyClause(" type A ")).toBe(true);
        expect(isTypeOnlyClause(" { type A, type B } ")).toBe(true);
    });

    it("keeps a value in the mix as a runtime import", () => {
        expect(isTypeOnlyClause(" { type A, b } ")).toBe(false);
        expect(isTypeOnlyClause(" { a } ")).toBe(false);
        expect(isTypeOnlyClause(" Default, { type A } ")).toBe(false);
        expect(isTypeOnlyClause(" * as ns ")).toBe(false);
        expect(isTypeOnlyClause(" typeof_helper ")).toBe(false);
    });
});

describe("readImports", () => {
    it("separates value, type-only and dynamic imports", () => {
        const src = `import { a } from "./a";
import type { B } from "./b";
import {
    type C,
    type D,
} from "./cd";
import "./side-effect";
export { e } from "./e";
export type { F } from "./f";
export * from "./star";
const lazy = () => import("./lazy");
const note = "import { fake } from './not-an-import'";
`;
        const got = readImports(src);
        expect(got.value.sort()).toEqual(["./a", "./e", "./side-effect", "./star"]);
        expect(got.type.sort()).toEqual(["./b", "./cd", "./f"]);
        expect(got.dynamic).toEqual(["./lazy"]);
    });

    it("does not read an import example in a comment or a string", () => {
        const src = `/*
import { V } from "../view/v";
*/
// import { W } from "./w";
const help = \`
import { X } from "./x";
use import("@/app/view/v") to load it
\`;
const url = "http://example.com/import('./not')";
import { real } from "./real";
`;
        const got = readImports(src);
        expect(got.value).toEqual(["./real"]);
        expect(got.type).toEqual([]);
        expect(got.dynamic).toEqual([]);
    });

    it("does not read across statements", () => {
        const src = `const x = 1
import type { T } from "./t"
import { v } from "./v"
`;
        const got = readImports(src);
        expect(got.type).toEqual(["./t"]);
        expect(got.value).toEqual(["./v"]);
    });
});

describe("resolveSpecifier", () => {
    const files = new Set([
        "frontend/app/a.ts",
        "frontend/app/b.tsx",
        "frontend/app/dir/index.ts",
        "frontend/util/u.ts",
        "frontend/cef-host-module.ts",
    ]);
    const at = (spec) => resolveSpecifier("frontend/app/view/x.ts", spec, files, ALIASES);

    it("resolves relative, aliased and index imports", () => {
        expect(at("../a")).toBe("frontend/app/a.ts");
        expect(at("../b")).toBe("frontend/app/b.tsx");
        expect(at("../dir")).toBe("frontend/app/dir/index.ts");
        expect(at("@/util/u")).toBe("frontend/util/u.ts");
        expect(at("@host-module")).toBe("frontend/cef-host-module.ts");
    });

    it("leaves packages and files that are not scanned", () => {
        expect(at("react")).toBeNull();
        expect(at("../missing")).toBeNull();
    });
});

describe("the import graph", () => {
    const sources = new Map([
        ["frontend/app/store/a.ts", `import { b } from "./b";`],
        ["frontend/app/store/b.ts", `import { c } from "./c";`],
        ["frontend/app/store/c.ts", `import { a } from "./a";`],
        ["frontend/app/store/typed.ts", `import type { T } from "./typed2";`],
        ["frontend/app/store/typed2.ts", `import { t } from "./typed";`],
        ["frontend/app/store/lazy.ts", `export const l = () => import("./lazy2");`],
        ["frontend/app/store/lazy2.ts", `import { l } from "./lazy";`],
        ["frontend/app/store/alone.ts", `import { self } from "./alone"; import { a } from "./a";`],
        ["frontend/app/view/v.ts", `export const v = 1;`],
        ["frontend/app/store/uses-view.ts", `import type { V } from "../view/v";`],
        ["frontend/util/u.ts", `import { v } from "@/app/view/v";`],
    ]);
    const graph = importGraph(sources, ALIASES);

    it("finds the files of a runtime cycle and no others", () => {
        const cyc = [...filesInCycles(graph.runtime)].sort();
        expect(cyc).toEqual(["frontend/app/store/a.ts", "frontend/app/store/b.ts", "frontend/app/store/c.ts"]);
    });

    it("does not count a type-only edge, a dynamic import or a self import as a cycle", () => {
        const cyc = filesInCycles(graph.runtime);
        for (const f of ["typed", "typed2", "lazy", "lazy2", "alone"]) expect(cyc.has(`frontend/app/store/${f}.ts`)).toBe(false);
        // The type-only and dynamic edges are still in the full graph.
        expect(graph.all.get("frontend/app/store/typed.ts")).toContain("frontend/app/store/typed2.ts");
        expect(graph.all.get("frontend/app/store/lazy.ts")).toContain("frontend/app/store/lazy2.ts");
    });

    it("finds a larger cycle that a node outside leads into", () => {
        const g = new Map([
            ["a", new Set(["b"])],
            ["b", new Set(["c"])],
            ["c", new Set(["a", "d"])],
            ["d", new Set()],
            ["e", new Set(["a"])],
        ]);
        expect([...filesInCycles(g)].sort()).toEqual(["a", "b", "c"]);
    });

    it("lists the imports from a lower layer into a higher one, type-only included", () => {
        expect([...invertedImports(graph.all)].sort()).toEqual([
            "frontend/app/store/uses-view.ts -> frontend/app/view/v.ts",
            "frontend/util/u.ts -> frontend/app/view/v.ts",
        ]);
    });
});

describe("comparison and update", () => {
    it("tells an added entry from a fixed one", () => {
        const got = compareSets(new Set(["x", "y"]), new Set(["y", "z"]));
        expect(got).toEqual({ added: ["x"], fixed: ["z"] });
    });

    it("tells a file that gained a reference from one that lost some", () => {
        const now = new Map([["a.rs", 3], ["b.rs", 1], ["new.rs", 1]]);
        const baseline = new Map([["a.rs", 2], ["b.rs", 2], ["gone.rs", 4]]);
        const got = compareCounts(now, baseline);
        expect(got.grew).toEqual([{ path: "a.rs", now: 3, allowed: 2 }, { path: "new.rs", now: 1, allowed: 0 }]);
        expect(got.shrank).toEqual([{ path: "b.rs", now: 1, allowed: 2 }, { path: "gone.rs", now: 0, allowed: 4 }]);
    });

    it("--update lowers and drops but never raises or adds", () => {
        const now = new Map([["a.rs", 3], ["b.rs", 1], ["new.rs", 1]]);
        const baseline = new Map([["a.rs", 2], ["b.rs", 2], ["gone.rs", 4]]);
        expect([...loweredCounts(now, baseline)]).toEqual([["a.rs", 2], ["b.rs", 1]]);
        expect([...loweredSet(new Set(["x", "n"]), new Set(["x", "z"]))]).toEqual(["x"]);
    });
});

describe("baseline files", () => {
    it("reads counts and plain lists, skipping comments", () => {
        expect([...parseCounts("# note\n\n3 a/b.rs\n12 c.rs\n")]).toEqual([["a/b.rs", 3], ["c.rs", 12]]);
        expect(parseLines("# note\nx\n\n  y  \n")).toEqual(["x", "y"]);
    });

    it("refuses a count line it cannot read", () => {
        expect(() => parseCounts("many a.rs", "f.baseline")).toThrow("f.baseline");
    });
});
