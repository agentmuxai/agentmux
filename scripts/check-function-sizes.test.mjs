// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-function-sizes.mjs: which functions it finds, how long it says
// they are, how they are named, and how the baseline moves.

import { describe, expect, it } from "vitest";
import { compare, lowered } from "./check-file-sizes.mjs";
import {
    LIMIT,
    closeOf,
    containerName,
    formatBaseline,
    functionsOver,
    parseBaseline,
    rustFunctions,
    scopeTo,
    tsFunctions,
} from "./check-function-sizes.mjs";
import { maskTs, rustCode } from "./lib/source-mask.mjs";

/** `count` filler statements, one per line. */
const filler = (count) => Array.from({ length: count }, (_, i) => `    step${i}();`).join("\n");

const rustNames = (src) => rustFunctions(rustCode(src)).map((f) => f.name);
const tsNames = (src) => tsFunctions(maskTs(src)).map((f) => f.name);

describe("closeOf", () => {
    it("finds the matching bracket and ignores other kinds", () => {
        const s = "{ a(1) [2] { b } }";
        expect(closeOf(s, 0)).toBe(s.length - 1);
        expect(closeOf("(((", 0)).toBe(-1);
    });
});

describe("containerName", () => {
    it("names the type of an impl or trait head", () => {
        expect(containerName(" Foo ")).toBe("Foo");
        expect(containerName("<T: Clone> Foo<T> ")).toBe("Foo");
        expect(containerName(" Display for Bar<'a, T> where T: Send ")).toBe("Bar");
        expect(containerName(" From<io::Error> for crate::Wrapper ")).toBe("Wrapper");
        expect(containerName(" Visitor: Send + Sync ")).toBe("Visitor");
    });
});

describe("rustFunctions", () => {
    it("finds functions with a body and skips declarations without one", () => {
        const src = "trait T { fn decl(&self) -> u8; fn dflt(&self) -> u8 { 1 } }\nfn free() {}\nextern \"C\" { fn ffi(x: u8); }\n";
        expect(rustNames(src)).toEqual(["T::dflt", "free"]);
    });

    it("names a method for the type of its impl, and the type after `for`", () => {
        const src = "impl<T> Foo<T> { pub fn new() -> Self { Foo } }\nimpl Display for Bar { fn fmt(&self) -> Result { Ok(()) } }\n";
        expect(rustNames(src)).toEqual(["Foo::new", "Bar::fmt"]);
    });

    it("names a function for the inline module, the function and the macro struct around it", () => {
        const src = [
            "mod inner { pub fn a() {} impl Foo { fn m(&self) {} } }",
            "fn outer() { struct Local; impl Local { fn go(&self) {} } fn helper() {} }",
            "wrap_task! { pub struct QuitTask {} impl Task { fn execute(&self) {} } }",
            "wrap_task! { pub struct OtherTask {} impl Task { fn execute(&self) {} } }",
            "",
        ].join("\n");
        expect(rustNames(src)).toEqual([
            "inner::a",
            "inner::Foo::m",
            "outer",
            "outer::Local::go",
            "outer::helper",
            "QuitTask::Task::execute",
            "OtherTask::Task::execute",
        ]);
    });

    it("skips an item that exists only when testing, also under cfg(all(test, ...))", () => {
        const src = [
            "#[cfg(all(test, windows))]",
            "mod win_tests { fn t() {} }",
            "#[cfg(all(unix, test))]",
            "fn only_unix_test() {}",
            "#[cfg(any(windows, test))]",
            "fn also_real() {}",
            "#[cfg(all(unix, not(test)))]",
            "fn not_a_test() {}",
            "",
        ].join("\n");
        expect(rustNames(src)).toEqual(["also_real", "not_a_test"]);
    });

    it("does not take `impl Trait` in a signature for an impl block", () => {
        const src = "fn a(x: impl Display) -> impl Iterator<Item = u8> { inner(); fn nested() {} }\n";
        expect(rustNames(src)).toEqual(["a", "a::nested"]);
    });

    it("reads generics, where clauses and return types before the body", () => {
        const src = [
            "pub async fn run<T, F>(f: F) -> Result<Vec<(u8, u8)>, Error>",
            "where",
            "    F: Fn(T) -> u8,",
            "{",
            "    f();",
            "}",
            "fn boxed() -> Box<dyn Fn(u8) -> u8> { Box::new(|x| x) }",
            "",
        ].join("\n");
        expect(rustNames(src)).toEqual(["run", "boxed"]);
    });

    it("ignores braces and fn text in strings and comments, and test-only code", () => {
        const src = [
            'fn a() { let s = "} fn fake() {"; /* fn also_fake() {} */ }',
            "#[cfg(test)]",
            "mod tests {",
            "    fn only_in_tests() {}",
            "}",
            "#[cfg(test)]",
            "fn test_helper() {}",
            "fn b() {}",
            "",
        ].join("\n");
        expect(rustNames(src)).toEqual(["a", "b"]);
    });
});

describe("tsFunctions", () => {
    it("reads a function declaration with a typed parameter and a return type", () => {
        const src = "export function Section(props: { account: Account }): JSX.Element {\n    return 1;\n}\n";
        expect(tsNames(src)).toEqual(["Section"]);
    });

    it("skips an overload signature and a statement that only looks like a call", () => {
        const src = "function f(a: string): void;\nfunction f(a: number): void;\nfunction f(a: any) {\n    go(a);\n}\nif (x) {\n    run();\n}\n";
        expect(tsNames(src)).toEqual(["f"]);
    });

    it("reads a const bound to an arrow function with a block or a parenthesised body", () => {
        const src = [
            "const a = (x: number): number => {",
            "    return x;",
            "};",
            "const View = (props: Props) => (",
            "    <div>{props.x}</div>",
            ");",
            "const plain = (x) => x + 1;",
            "const single = x => {",
            "    use(x);",
            "};",
            "",
        ].join("\n");
        expect(tsNames(src)).toEqual(["a", "View", "single"]);
    });

    it("reads a function passed to memo, forwardRef or useCallback", () => {
        const src = [
            "const A = React.memo((props: P) => {",
            "    return null;",
            "});",
            "const B = forwardRef<HTMLDivElement, P>(function B(props, ref) {",
            "    return null;",
            "});",
            "const C = useCallback(async () => {",
            "    await go();",
            "}, []);",
            "const D: React.FC<P> = ({ x }) => {",
            "    return x;",
            "};",
            "",
        ].join("\n");
        expect(tsNames(src)).toEqual(["A", "B", "C", "D"]);
    });

    it("reads class methods under the class name, and properties bound to functions", () => {
        const src = [
            "class Model extends Base<Props> {",
            "    constructor(opts: Opts) {",
            "        super();",
            "    }",
            "    private async load<T>(id: string): Promise<{ ok: boolean }> {",
            "        return null;",
            "    }",
            "    handle = (e: Event) => {",
            "        run(e);",
            "    };",
            "    get size(): number {",
            "        return 1;",
            "    }",
            "}",
            "const api = {",
            "    open(path: string) {",
            "        go(path);",
            "    },",
            "    close: () => {",
            "        stop();",
            "    },",
            "    name: 'x',",
            "};",
            "",
        ].join("\n");
        expect(tsNames(src).sort()).toEqual(["Model::constructor", "Model::handle", "Model::load", "Model::size", "close", "open"].sort());
    });

    it("reads a class field with a type annotation before its function", () => {
        const src = [
            "class Model {",
            "    handle: () => void = () => {",
            "        run();",
            "    };",
            "    other: Map<string, number> = new Map();",
            "    kind: string = 'a';",
            "}",
            "",
        ].join("\n");
        expect(tsNames(src)).toEqual(["Model::handle"]);
    });

    it("counts a function once when two forms read it, and not a callback with no name", () => {
        const src = "const f = function g() {\n    one();\n};\nitems.forEach((x) => {\n    two(x);\n});\n";
        expect(tsNames(src)).toEqual(["f"]);
    });

    it("keeps the end of a component whose JSX text has a lone parenthesis", () => {
        const src = [
            "export const Big = () => (",
            "    <div>",
            "        Progress (",
            "        <b>x</b>",
            "        1) done",
            "    </div>",
            ");",
            "function after() {}",
            "",
        ].join("\n");
        const m = maskTs(src, { jsx: true });
        const fns = tsFunctions(m);
        expect(fns.map((f) => f.name)).toEqual(["Big", "after"]);
        expect(m.slice(fns[0].end, fns[0].end + 1)).toBe(")");
        expect(m.slice(fns[0].start, fns[0].end + 1).split("\n").length).toBe(7);
    });

    it("keeps the end of a component after an apostrophe or a closing tag in its JSX", () => {
        const src = [
            "export function Section(): JSX.Element {",
            "    return (",
            "        <Show fallback={<p>Open AgentMux's app.</p>}>",
            "            <For each={xs}>{(x) => <Row item={x} />}</For>",
            "        </Show>",
            "    );",
            "}",
            "function after() {}",
            "",
        ].join("\n");
        const m = maskTs(src);
        const fns = tsFunctions(m);
        expect(fns.map((f) => f.name)).toEqual(["Section", "after"]);
        expect(m.slice(fns[0].end, fns[0].end + 1)).toBe("}");
        expect(m.slice(fns[0].start, fns[0].end).split("\n").length).toBe(7);
    });
});

describe("functionsOver", () => {
    const rust = (n) => rustCode(`fn long() {\n${filler(n)}\n}\nfn short() {}\n`);

    it("counts the lines from the fn line to the closing brace, and lists only what passes the limit", () => {
        expect(functionsOver(rust(LIMIT - 3), "rust")).toEqual([]);
        expect(functionsOver(rust(LIMIT - 2), "rust")).toEqual([]);
        expect(functionsOver(rust(LIMIT - 1), "rust")).toEqual([{ name: "long", lines: LIMIT + 1 }]);
    });

    it("tells two functions of one name apart by order", () => {
        const src = `fn dup() {\n${filler(LIMIT)}\n}\nfn dup() {\n${filler(LIMIT + 5)}\n}\n`;
        expect(functionsOver(rustCode(src), "rust")).toEqual([
            { name: "dup", lines: LIMIT + 2 },
            { name: "dup#2", lines: LIMIT + 7 },
        ]);
    });

    it("keeps the number of a function when an earlier one of its name is under the limit", () => {
        const src = `fn dup() {}\nfn dup() {\n${filler(LIMIT)}\n}\n`;
        expect(functionsOver(rustCode(src), "rust")).toEqual([{ name: "dup#2", lines: LIMIT + 2 }]);
        const shrunk = `fn dup() {\n${filler(LIMIT - 3)}\n}\nfn dup() {\n${filler(LIMIT)}\n}\n`;
        expect(functionsOver(rustCode(shrunk), "rust")).toEqual([{ name: "dup#2", lines: LIMIT + 2 }]);
    });

    it("reads TypeScript the same way", () => {
        const src = `export const Big = (): number => {\n${filler(LIMIT)}\n    return 1;\n};\n`;
        expect(functionsOver(maskTs(src), "ts")).toEqual([{ name: "Big", lines: LIMIT + 3 }]);
    });

    it("counts a nested function on its own and inside the one around it", () => {
        const src = `fn outer() {\n    fn inner() {\n${filler(LIMIT)}\n    }\n}\n`;
        expect(functionsOver(rustCode(src), "rust")).toEqual([
            { name: "outer", lines: LIMIT + 4 },
            { name: "outer::inner", lines: LIMIT + 2 },
        ]);
    });
});

describe("baseline file", () => {
    it("round-trips and sorts", () => {
        const text = formatBaseline(
            new Map([
                ["b.rs::Type::run", 120],
                ["a.ts::go", 101],
            ])
        );
        expect([...parseBaseline(text)]).toEqual([
            ["a.ts::go", 101],
            ["b.rs::Type::run", 120],
        ]);
    });

    it("rejects a line that is not `<lines> <path>::<name>`", () => {
        expect(() => parseBaseline("12 just/a/path\n")).toThrow(/expected/);
    });
});

describe("the ratchet", () => {
    const baseline = new Map([
        ["a.rs::big", 300],
        ["a.rs::shrunk", 200],
        ["a.rs::gone", 150],
    ]);

    it("fails a function that grew or is new over the limit, and one that got shorter or is gone", () => {
        const now = new Map([
            ["a.rs::big", 301],
            ["a.rs::shrunk", 150],
            ["a.rs::new", 101],
        ]);
        const got = compare(now, baseline, LIMIT);
        expect(got.grew.map((f) => f.path)).toEqual(["a.rs::big"]);
        expect(got.added.map((f) => f.path)).toEqual(["a.rs::new"]);
        expect(got.shrank.map((f) => f.path)).toEqual(["a.rs::shrunk"]);
        expect(got.gone.map((f) => f.path)).toEqual(["a.rs::gone"]);
    });

    it("checks only the files in scope, so a change that touches none can't fail on drift", () => {
        const drifted = new Map([
            ["a.rs::big", 305],
            ["b.rs::other", 120],
        ]);
        const known = new Map([
            ["a.rs::big", 300],
            ["b.rs::other", 120],
        ]);
        expect(compare(drifted, known, LIMIT).grew.map((f) => f.path)).toEqual(["a.rs::big"]);
        const touchedB = new Set(["b.rs"]);
        const got = compare(scopeTo(drifted, touchedB), scopeTo(known, touchedB), LIMIT);
        expect(got.grew).toEqual([]);
        expect([...scopeTo(drifted, null)]).toEqual([...drifted]);
        const touchedA = new Set(["a.rs"]);
        expect(compare(scopeTo(drifted, touchedA), scopeTo(known, touchedA), LIMIT).grew.map((f) => f.path)).toEqual(["a.rs::big"]);
    });

    it("lowers on --update and never raises or adds", () => {
        const now = new Map([
            ["a.rs::big", 400],
            ["a.rs::shrunk", 150],
            ["a.rs::new", 101],
        ]);
        expect([...lowered(now, baseline, LIMIT)]).toEqual([
            ["a.rs::big", 300],
            ["a.rs::shrunk", 150],
        ]);
    });

    it("moves only the entries in scope", () => {
        const now = new Map([["a.rs::shrunk", 150]]);
        const got = lowered(now, baseline, LIMIT, new Set(["a.rs::shrunk"]));
        expect(got.get("a.rs::shrunk")).toBe(150);
        expect(got.get("a.rs::big")).toBe(300);
        expect(got.get("a.rs::gone")).toBe(150);
    });
});
